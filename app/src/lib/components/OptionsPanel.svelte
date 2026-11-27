<script lang="ts">
  // The options of one enabled module, edited as a draft and applied like a
  // module change (servers stopped). Used by "Server settings" and "Bots".
  import { untrack } from "svelte";
  import {
    api,
    asCommandError,
    type CommandError,
    type MaintenanceAction,
    type MaintenanceOutcome,
    type ModuleOption,
    type ModuleOptions,
    type OptionValue,
  } from "../api";
  import { localized, t } from "../i18n/i18n.svelte";
  import { runOperation } from "../launcher.svelte";
  import ErrorBox from "./ErrorBox.svelte";
  import Modal from "./Modal.svelte";

  let {
    instanceId,
    module,
    title,
    locked,
    onerror,
    columns = 1,
  }: {
    instanceId: string;
    module: ModuleOptions;
    title: string;
    locked: boolean;
    onerror: (error: CommandError | null) => void;
    /** Options side by side (long lists). */
    columns?: number;
  } = $props();

  function asText(value: OptionValue): string {
    return String(value);
  }

  function saved(): Record<string, string> {
    return Object.fromEntries(module.options.map((o) => [o.option.id, asText(o.value)]));
  }

  /** Edited values, as text, reset whenever the saved values change. */
  let draft = $state<Record<string, string>>({});
  const savedKey = $derived(JSON.stringify(module.options.map((o) => o.value)));
  $effect(() => {
    void savedKey;
    untrack(() => (draft = saved()));
  });

  /** Some options apply SQL files: turning one off rebuilds a database. */
  const hasSql = $derived(module.options.some((o) => o.option.type === "sql"));
  /** Applying the draft turns off a file in force: its database is rebuilt. */
  const rebuilds = $derived(
    module.options.some(
      (o) => o.option.type === "sql" && o.value === true && draft[o.option.id] === "false",
    ),
  );

  const changed = $derived(module.options.some((o) => draft[o.option.id] !== asText(o.value)));

  /** Whether the option's requirement holds in the draft (none: always). */
  function met(option: ModuleOption, values: Record<string, string> = draft): boolean {
    const requires = option.type === "sql" ? option.requires : null;
    return !requires || requires.values.includes(values[requires.option] ?? "");
  }

  /** "Needs <option>: <values>", with the required option's labels. */
  function requirement(option: ModuleOption): string | null {
    const requires = option.type === "sql" ? option.requires : null;
    if (!requires) return null;
    const target = module.options.find((o) => o.option.id === requires.option)?.option;
    if (!target) return null;
    const values = requires.values.map((v) => {
      const choice = target.type === "choice" ? target.choices.find((c) => c.id === v) : undefined;
      return choice ? localized(choice.label) : v;
    });
    return t("settings.requires", { option: localized(target.label), values: values.join(" / ") });
  }

  /** The realm's server build has what the option needs. */
  function available(option: ModuleOption): boolean {
    return module.options.find((o) => o.option.id === option.id)?.available ?? true;
  }

  function set(option: string, value: string) {
    const next = { ...draft, [option]: value };
    // Options whose requirement no longer holds turn off with it.
    for (const { option: o } of module.options) {
      if (next[o.id] === "true" && !met(o, next)) next[o.id] = "false";
    }
    draft = next;
  }

  /** The options under their headings: those in no group first, unheaded. */
  const sections = $derived.by(() => {
    const groups = module.groups ?? [];
    const grouped = new Set(groups.flatMap((g) => g.options));
    const pick = (ids: string[]) =>
      ids.flatMap((id) => module.options.filter((o) => o.option.id === id).map((o) => o.option));
    return [
      { id: "", label: null, options: module.options.map((o) => o.option).filter((o) => !grouped.has(o.id)) },
      ...groups.map((g) => ({ id: g.id, label: localized(g.label), options: pick(g.options) })),
    ].filter((s) => s.options.length > 0);
  });

  function help(option: ModuleOption): string | null {
    return option.help ? localized(option.help) : null;
  }

  /** The cleanup the player opened (ADR 0030): confirm, then its result. */
  let cleanup = $state<MaintenanceAction | null>(null);
  let cleaning = $state(false);
  let cleaned = $state<MaintenanceOutcome | null>(null);
  let cleanupError = $state<CommandError | null>(null);
  function openCleanup(action: MaintenanceAction) {
    cleanup = action;
    cleaned = null;
    cleanupError = null;
  }
  async function runCleanup() {
    if (!cleanup) return;
    cleaning = true;
    cleanupError = null;
    try {
      cleaned = await api.runMaintenance(instanceId, module.module, cleanup.id);
    } catch (raw) {
      cleanupError = asCommandError(raw);
    } finally {
      cleaning = false;
    }
  }

  async function apply() {
    const values = module.options
      .filter((o) => o.option.id in draft && draft[o.option.id] !== asText(o.value))
      .map((o) => [o.option.id, draft[o.option.id]] as [string, string]);
    onerror(null);
    try {
      await runOperation(instanceId, () => api.setOptions(instanceId, module.module, values));
    } catch (raw) {
      onerror(asCommandError(raw));
    }
  }
</script>

<section class="panel">
  <h3 class="panel-title">{title}</h3>
  <!-- One scroll for every group when the panel is given a height. -->
  <div class="sections">
    {#each sections as section (section.id)}
      {#if section.label}<h4 class="group">{section.label}</h4>{/if}
      <ul class="options" style:--columns={columns}>
        {#each section.options as option (option.id)}
          {@const value = draft[option.id] ?? ""}
          <li title={help(option) ?? undefined}>
            <span class="label">
              {localized(option.label)}
              {#if help(option)}<span class="help" aria-hidden="true">?</span>{/if}
              {#if !available(option)}
                <span class="needs">{t("settings.needsServerUpdate")}</span>
              {:else if !met(option)}<span class="needs">{requirement(option)}</span>{/if}
            </span>
            {#if option.type === "boolean" || option.type === "sql"}
              <input
                type="checkbox"
                disabled={locked || ((!met(option) || !available(option)) && value !== "true")}
                checked={value === "true"}
                onchange={(e) => set(option.id, String(e.currentTarget.checked))}
              />
            {:else if option.type === "integer" || option.type === "number"}
              <input
                type="number"
                disabled={locked}
                min={option.min ?? undefined}
                max={option.max ?? undefined}
                step={option.type === "number" ? "0.1" : "1"}
                {value}
                onchange={(e) => set(option.id, e.currentTarget.value)}
              />
            {:else}
              <select disabled={locked} {value} onchange={(e) => set(option.id, e.currentTarget.value)}>
                {#each option.choices as choice (choice.id)}
                  <option value={choice.id}>{localized(choice.label)}</option>
                {/each}
              </select>
            {/if}
          </li>
        {/each}
      </ul>
    {/each}
    {#if module.maintenance?.length}
      <h4 class="group">{t("maintenance.title")}</h4>
      <ul class="options">
        {#each module.maintenance as action (action.id)}
          <li title={action.help ? localized(action.help) : undefined}>
            <span class="label">
              {localized(action.label)}
              {#if action.help}<span class="help" aria-hidden="true">?</span>{/if}
              {#if locked}<span class="needs">{t("maintenance.stopFirst")}</span>{/if}
            </span>
            <button class="btn small" disabled={locked} onclick={() => openCleanup(action)}>
              {t("maintenance.run")}
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
  <div class="apply">
    <p class="muted">{t("settings.optionsHint")}</p>
    {#if rebuilds}
      <p class="warn">{t("settings.sqlRebuild")}</p>
    {:else if hasSql}
      <p class="muted">{t("settings.sqlHint")}</p>
    {/if}
    <button class="btn" disabled={!changed || locked} onclick={() => (draft = saved())}>
      {t("common.reset")}
    </button>
    <button class="btn btn-play" disabled={!changed || locked} onclick={apply}>{t("common.apply")}</button>
  </div>
</section>

{#if cleanup}
  <Modal title={localized(cleanup.label)} onclose={() => !cleaning && (cleanup = null)}>
    {#if cleaned}
      <p>{t("maintenance.done", { count: String(cleaned.removed), backup: cleaned.backup })}</p>
    {:else}
      {#if cleanup.help}<p class="muted">{localized(cleanup.help)}</p>{/if}
      <p class="danger-text">{localized(cleanup.warning)}</p>
      {#if cleaning}<p class="muted" role="status">{t("maintenance.working")}</p>{/if}
      {#if cleanupError}
        <ErrorBox error={cleanupError} />
      {/if}
    {/if}
    {#snippet actions()}
      {#if cleaned}
        <button class="btn" onclick={() => (cleanup = null)}>{t("common.close")}</button>
      {:else}
        <button class="btn" disabled={cleaning} onclick={() => (cleanup = null)}>{t("common.cancel")}</button>
        <button class="btn danger" disabled={cleaning} onclick={runCleanup}>{t("maintenance.confirm")}</button>
      {/if}
    {/snippet}
  </Modal>
{/if}

<style>
  /* Fills its column, the buttons at the bottom; never shorter than its
     options (several panels on a page: the page scrolls, none overlaps). */
  section {
    flex: 1 0 auto;
    display: flex;
    flex-direction: column;
  }

  /* Grows with its options; when the page gives the panel a height
     (module options beside the module list), the groups scroll together
     and the buttons stay in view. */
  .sections {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding-right: var(--space-1);
  }

  .options {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(var(--columns), minmax(0, 1fr));
    column-gap: var(--space-6);
  }

  .group {
    margin: var(--space-4) 0 var(--space-1);
    font-size: 0.8rem;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--accent);
  }

  .options li {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-height: 38px;
    padding: var(--space-1) 0;
    border-bottom: 1px solid var(--border);
  }

  .label {
    flex: 1;
    min-width: 0;
    font-size: 0.92rem;
    line-height: 1.3;
  }

  /* The help text is the row's tooltip. */
  .help {
    display: inline-grid;
    place-items: center;
    width: 15px;
    height: 15px;
    margin-left: var(--space-1);
    border: 1px solid var(--frame-light);
    border-radius: 50%;
    font-size: 0.65rem;
    color: var(--accent);
    vertical-align: 1px;
    cursor: help;
  }

  /* Why a box is greyed out: always visible, under the label. */
  .needs {
    display: block;
    font-size: 0.78rem;
    color: var(--text-muted);
  }

  .btn.small {
    padding: 2px var(--space-3);
    font-size: 0.82rem;
  }

  .btn.danger {
    border-color: var(--danger);
    color: var(--danger);
  }

  .danger-text {
    color: var(--danger);
  }

  input[type="number"] {
    width: 6rem;
  }

  select {
    max-width: 55%;
  }

  /* Hint on its own line, buttons under it on the right: never squeezed. */
  .apply {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    align-items: center;
    gap: var(--space-2) var(--space-3);
    margin-top: auto;
    padding-top: var(--space-3);
  }

  .apply p {
    flex: 1 1 100%;
    margin: 0;
    font-size: 0.82rem;
  }

  .apply p.warn {
    color: var(--danger);
  }

  .apply .btn {
    padding: var(--space-1) var(--space-3);
    font-size: 0.85rem;
  }

  .apply .btn-play {
    min-width: 8rem;
    font-size: 0.95rem;
  }
</style>
