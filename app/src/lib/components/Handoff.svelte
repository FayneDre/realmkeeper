<script lang="ts">
  // Playing a realm on several computers (ADR 0028): this computer's
  // handoff folder (one the player already synchronises: Google Drive,
  // OneDrive, Dropbox), whether the shown realm is shared, and the realms
  // other computers shared there, to install here.
  import {
    api,
    asCommandError,
    pickFolder,
    type CommandError,
    type EmulatorOverview,
    type HandoffSettings,
    type HandoffStatus,
    type InstanceView,
    type SharedRealm,
  } from "../api";
  import { t } from "../i18n/i18n.svelte";
  import { chooseRealm, launcher, realmLabel, runOperation } from "../launcher.svelte";
  import Modal from "./Modal.svelte";
  import Notify from "./Notify.svelte";

  let {
    realm,
    instance,
    newId,
    onclose,
  }: {
    realm: EmulatorOverview;
    instance: InstanceView | null;
    /** A free realm id made from a name. */
    newId: (base: string) => string;
    onclose: () => void;
  } = $props();

  const busy = $derived(launcher.progress !== null);
  const running = $derived(instance ? launcher.isRunning(instance.record.id) : false);
  const ready = $derived(!!instance && instance.installed && !instance.unfinished);

  let settings = $state<HandoffSettings | null>(null);
  let status = $state<HandoffStatus | null>(null);
  let shared = $state<SharedRealm[] | null>(null);
  let error = $state<CommandError | null>(null);
  let note = $state<string | null>(null);
  let working = $state(false);

  async function load() {
    try {
      settings = await api.handoffSettings();
      status = instance ? await api.handoffStatus(instance.record.id) : null;
      shared = settings.folder ? await api.handoffShared().catch(() => null) : null;
    } catch (raw) {
      error = asCommandError(raw);
    }
  }

  $effect(() => {
    void launcher.handoffTick;
    void instance?.record.id;
    void load();
  });

  async function act(run: () => Promise<unknown>) {
    error = null;
    note = null;
    working = true;
    try {
      await run();
    } catch (raw) {
      error = asCommandError(raw);
    } finally {
      working = false;
      await load();
    }
  }

  const useFolder = (folder: string | null, create = false) =>
    act(async () => {
      settings = await api.setHandoffFolder(folder, create);
    });

  function chooseFolder() {
    void act(async () => {
      const folder = await pickFolder(t("handoff.pickTitle"));
      if (folder) settings = await api.setHandoffFolder(folder);
    });
  }

  function share() {
    if (!instance) return;
    const id = instance.record.id;
    void act(async () => {
      await runOperation(id, () => api.handoffShare(id), "handoff.sending");
      note = t("handoff.shared");
    });
  }

  const unshare = () => {
    if (!instance) return;
    const id = instance.record.id;
    void act(() => api.handoffUnshare(id));
  };

  function sendNow() {
    if (!instance) return;
    const id = instance.record.id;
    void act(async () => {
      await runOperation(id, () => api.handoffSend(id, false), "handoff.sending");
      note = t("handoff.sentToast");
    });
  }

  function join(item: SharedRealm) {
    const name = item.name;
    const id = newId(name ?? realm.id);
    void act(async () => {
      await runOperation(id, () => api.handoffJoin(item.uid, id, name), "handoff.joining");
      chooseRealm(item.emulator, id);
      note = t("handoff.joined", { name: name ?? id });
    });
  }

  const when = (secs: number) =>
    new Date(secs * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });

  const statusText = $derived.by(() => {
    if (!status) return "";
    switch (status.state) {
      case "notShared":
        return t("handoff.state.notShared");
      case "noFolder":
        return t("handoff.state.noFolder");
      case "unavailable":
        return t("handoff.state.unavailable");
      case "upToDate":
        return status.sent ? t("handoff.state.sent", { date: when(status.sent) }) : t("handoff.state.upToDate");
      case "toSend":
        return t("handoff.state.toSend");
      case "toTake":
        return t("handoff.toTakeBody", { machine: status.from.machine, date: when(status.from.secs) });
      case "arriving":
        return t("handoff.arriving", { machine: status.from.machine });
      case "conflict":
        return t("handoff.conflictBody", { machine: status.from.machine, date: when(status.from.secs) });
    }
    return "";
  });

  const others = $derived((shared ?? []).filter((s) => s.emulator === realm.id && !s.local));
</script>

<Modal title={t("handoff.title")} {onclose} width="620px">
  <p class="muted">{t("handoff.body")}</p>

  <h4>{t("handoff.folder")}</h4>
  {#if settings}
    {#if settings.folder}
      <p class="folder"><code>{settings.folder}</code></p>
      <div class="row">
        <button class="btn small" disabled={working} onclick={chooseFolder}>{t("handoff.changeFolder")}</button>
        <button class="btn small" disabled={working} onclick={() => void useFolder(null)}>
          {t("handoff.noFolder")}
        </button>
      </div>
    {:else}
      <p>{t("handoff.chooseFolder")}</p>
      <div class="row">
        {#each settings.suggestions as suggestion (suggestion.path)}
          <button class="btn small" disabled={working} onclick={() => void useFolder(suggestion.path, true)}>
            {t("handoff.useSuggestion", { client: suggestion.client })}
          </button>
        {/each}
        <button class="btn small" disabled={working} onclick={chooseFolder}>{t("handoff.pickFolder")}</button>
      </div>
    {/if}
    <p class="hint muted">{t("handoff.personalOnly", { machine: settings.machine })}</p>
  {:else}
    <p class="muted">{t("modules.loading")}</p>
  {/if}

  {#if instance && settings?.folder}
    <h4>{t("handoff.thisRealm", { name: realmLabel(instance) })}</h4>
    <p>{statusText}</p>
    <div class="row">
      {#if status?.state === "notShared"}
        <button
          class="btn small"
          disabled={working || busy || running || !ready}
          title={running ? t("realms.stopFirst") : undefined}
          onclick={share}
        >
          {t("handoff.share")}
        </button>
      {:else if status}
        {#if status.state === "toSend"}
          <button class="btn small" disabled={working || busy || running || !ready} onclick={sendNow}>
            {t("handoff.sendNow")}
          </button>
        {/if}
        <button class="btn small" disabled={working || busy} onclick={unshare}>{t("handoff.unshare")}</button>
      {/if}
    </div>
  {/if}

  {#if settings?.folder}
    <h4>{t("handoff.inFolder")}</h4>
    {#if shared === null}
      <p class="muted">{t("handoff.state.unavailable")}</p>
    {:else if others.length === 0}
      <p class="muted">{t("handoff.noneToInstall")}</p>
    {:else}
      <ul class="shared">
        {#each others as item (item.uid)}
          <li>
            <span>
              <strong>{item.name ?? t("realms.defaultName")}</strong>
              <small class="muted">
                {t("handoff.sharedFrom", { machine: item.from.machine, date: when(item.from.secs) })}
              </small>
            </span>
            <button
              class="btn small"
              disabled={working || busy || !item.arrived}
              title={item.arrived ? undefined : t("handoff.arriving", { machine: item.from.machine })}
              onclick={() => join(item)}
            >
              {t("handoff.install")}
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
</Modal>
<Notify text={note} />
<Notify {error} />

<style>
  h4 {
    margin: var(--space-4) 0 var(--space-2);
    color: var(--title);
    font-family: var(--font-display);
    font-weight: normal;
  }

  .folder code {
    word-break: break-all;
  }

  .row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .hint {
    margin-top: var(--space-2);
    font-size: 0.85rem;
  }

  .btn.small {
    padding: var(--space-1) var(--space-3);
    font-size: 0.8rem;
  }

  .shared {
    display: grid;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .shared li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .shared span {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
</style>
