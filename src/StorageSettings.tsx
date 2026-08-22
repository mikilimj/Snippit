import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import {
  storageConnectSftp,
  storageConnectOauth,
  storageDisconnect,
  storageStatus,
  syncNow,
  getSyncSettings,
  setSyncSettings,
  type StorageStatus,
  type SyncSettings,
} from "./api";
import { percentIsValid } from "./syncDiff";

type OauthProvider = "dropbox" | "drive";
type Provider = "sftp" | OauthProvider;

function providerLabel(provider: string): string {
  switch (provider) {
    case "sftp":
      return "SFTP server";
    case "dropbox":
      return "Dropbox";
    case "drive":
      return "Google Drive";
    default:
      return provider;
  }
}

/**
 * Provider-connect flow: three provider choices, then either the SFTP form
 * or an OAuth sign-in button. Fully self-contained (owns its own pending/
 * error state) so it can be reused standalone — Task 11's wizard restore
 * flow mounts this same component and just needs a callback for success.
 */
export function StorageConnectForm({ onConnected }: { onConnected: () => void }) {
  const [provider, setProvider] = useState<Provider | null>(null);

  // SFTP fields
  const [host, setHost] = useState("");
  const [port, setPort] = useState(22);
  const [user, setUser] = useState("");
  const [password, setPassword] = useState("");
  const [keyFile, setKeyFile] = useState("");
  const [sftpBasePath, setSftpBasePath] = useState("");

  // OAuth base path (shared field, reset when switching provider)
  const [oauthBasePath, setOauthBasePath] = useState("");

  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function selectProvider(p: Provider) {
    setProvider(p);
    setError(null);
    setOauthBasePath("");
  }

  async function pickKeyFile() {
    const file = await open({ multiple: false, title: "Select SSH private key" });
    if (typeof file === "string") setKeyFile(file);
  }

  async function connectSftp() {
    setPending(true);
    setError(null);
    try {
      await storageConnectSftp({
        host: host.trim(),
        port,
        user: user.trim(),
        password: password ? password : undefined,
        keyFile: keyFile ? keyFile : undefined,
        basePath: sftpBasePath.trim() ? sftpBasePath.trim() : undefined,
      });
      onConnected();
    } catch (e) {
      setError(String(e));
    } finally {
      setPending(false);
    }
  }

  async function connectOauth(p: OauthProvider) {
    setPending(true);
    setError(null);
    try {
      await storageConnectOauth(p, oauthBasePath.trim() ? oauthBasePath.trim() : undefined);
      onConnected();
    } catch (e) {
      setError(String(e));
    } finally {
      setPending(false);
    }
  }

  return (
    <div className="storage-connect">
      <div className="storage-provider-picker">
        <button
          type="button"
          className={`storage-provider-btn${provider === "sftp" ? " active" : ""}`}
          onClick={() => selectProvider("sftp")}
        >
          SFTP server
        </button>
        <button
          type="button"
          className={`storage-provider-btn${provider === "dropbox" ? " active" : ""}`}
          onClick={() => selectProvider("dropbox")}
        >
          Dropbox
        </button>
        <button
          type="button"
          className={`storage-provider-btn${provider === "drive" ? " active" : ""}`}
          onClick={() => selectProvider("drive")}
        >
          Google Drive
        </button>
      </div>

      {provider === "sftp" && (
        <div className="storage-form">
          <label className="storage-field">
            <span>Host</span>
            <input type="text" value={host} onChange={e => setHost(e.target.value)} placeholder="example.com" />
          </label>
          <label className="storage-field storage-field-port">
            <span>Port</span>
            <input
              type="number"
              value={port}
              onChange={e => setPort(e.target.value === "" ? 22 : Number(e.target.value))}
            />
          </label>
          <label className="storage-field">
            <span>Username</span>
            <input type="text" value={user} onChange={e => setUser(e.target.value)} />
          </label>
          <label className="storage-field">
            <span>Password</span>
            <input
              type="password"
              value={password}
              onChange={e => setPassword(e.target.value)}
              placeholder="or use a key file below"
            />
          </label>
          <div className="storage-actions-row">
            <button type="button" className="btn-sm" onClick={pickKeyFile}>
              {keyFile ? "Key file selected" : "Choose key file…"}
            </button>
            {keyFile && (
              <span className="settings-path storage-keyfile-path" title={keyFile}>
                {keyFile}
              </span>
            )}
          </div>
          <label className="storage-field">
            <span>Base path (optional)</span>
            <input
              type="text"
              value={sftpBasePath}
              onChange={e => setSftpBasePath(e.target.value)}
              placeholder="/remote/clips"
            />
          </label>
          <div className="settings-actions">
            <button
              className="btn-export"
              onClick={connectSftp}
              disabled={pending || !host.trim() || !user.trim()}
            >
              {pending ? "Connecting…" : "Test & Connect"}
            </button>
          </div>
        </div>
      )}

      {(provider === "dropbox" || provider === "drive") && (
        <div className="storage-form">
          <label className="storage-field">
            <span>Base path (optional)</span>
            <input
              type="text"
              value={oauthBasePath}
              onChange={e => setOauthBasePath(e.target.value)}
              placeholder="/Snippit"
            />
          </label>
          <div className="settings-actions">
            <button className="btn-export" onClick={() => connectOauth(provider)} disabled={pending}>
              {pending ? "Waiting for browser approval…" : `Sign in with ${provider === "dropbox" ? "Dropbox" : "Google"}`}
            </button>
          </div>
        </div>
      )}

      {error && <div className="msg-error">{error}</div>}
    </div>
  );
}

type SizeMode = "full" | "75" | "50" | "25" | "custom";

function percentToMode(p: number | null): SizeMode {
  if (p === null) return "full";
  if (p === 75) return "75";
  if (p === 50) return "50";
  if (p === 25) return "25";
  return "custom";
}

function SizeSelect({
  label,
  mode,
  custom,
  onModeChange,
  onCustomChange,
  warning,
}: {
  label: string;
  mode: SizeMode;
  custom: string;
  onModeChange: (m: SizeMode) => void;
  onCustomChange: (v: string) => void;
  warning?: string;
}) {
  const customNum = Number(custom);
  const customInvalid = mode === "custom" && custom.trim() !== "" && !percentIsValid(customNum);
  return (
    <div className="storage-size-row">
      <span className="export-size-label">{label}</span>
      <select className="export-size-select" value={mode} onChange={e => onModeChange(e.target.value as SizeMode)}>
        <option value="full">Full size</option>
        <option value="75">75%</option>
        <option value="50">50%</option>
        <option value="25">25%</option>
        <option value="custom">Custom…</option>
      </select>
      {mode === "custom" && (
        <input
          className="export-size-custom"
          type="number"
          min={1}
          max={99}
          value={custom}
          onChange={e => onCustomChange(e.target.value)}
        />
      )}
      {customInvalid && <span className="storage-size-invalid">1-99 only</span>}
      {warning && <p className="settings-hint storage-size-warning">{warning}</p>}
    </div>
  );
}

/**
 * Full "Storage" settings tab: connect/connected state, size settings, tray
 * checkbox and the entry point into the diff scan (Task 9 owns the modal
 * itself — this just calls back out through `onOpenDiff`).
 */
export function StorageSection({ onOpenDiff }: { onOpenDiff?: () => void }) {
  const [status, setStatus] = useState<StorageStatus | null | undefined>(undefined);
  const [settings, setSettingsState] = useState<SyncSettings | null>(null);

  const [uploadMode, setUploadMode] = useState<SizeMode>("full");
  const [localMode, setLocalMode] = useState<SizeMode>("full");
  const [uploadCustom, setUploadCustom] = useState("");
  const [localCustom, setLocalCustom] = useState("");
  const [settingsMsg, setSettingsMsg] = useState<string | null>(null);

  const [syncingNow, setSyncingNow] = useState(false);
  const [disconnecting, setDisconnecting] = useState(false);

  function refreshStatus() {
    storageStatus()
      .then(setStatus)
      .catch(() => setStatus(null));
  }

  useEffect(() => {
    refreshStatus();
    getSyncSettings()
      .then(s => {
        setSettingsState(s);
        setUploadMode(percentToMode(s.uploadSizePercent));
        setLocalMode(percentToMode(s.localSizePercent));
        if (s.uploadSizePercent != null && percentToMode(s.uploadSizePercent) === "custom") {
          setUploadCustom(String(s.uploadSizePercent));
        }
        if (s.localSizePercent != null && percentToMode(s.localSizePercent) === "custom") {
          setLocalCustom(String(s.localSizePercent));
        }
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    const unStatus = listen<StorageStatus>("sync-status", e => setStatus(e.payload));
    return () => {
      unStatus.then(f => f());
    };
  }, []);

  async function saveSettings(partial: Partial<SyncSettings>) {
    setSettingsMsg(null);
    if (!settings) return;
    const next = { ...settings, ...partial };
    setSettingsState(next);
    try {
      await setSyncSettings(next);
    } catch (e) {
      setSettingsMsg(`Couldn't save size settings: ${e}`);
    }
  }

  function handleUploadModeChange(m: SizeMode) {
    setUploadMode(m);
    if (m === "custom") {
      setUploadCustom(settings?.uploadSizePercent != null ? String(settings.uploadSizePercent) : "");
      return;
    }
    saveSettings({ uploadSizePercent: m === "full" ? null : Number(m) });
  }

  function handleUploadCustomChange(v: string) {
    setUploadCustom(v);
    const n = Number(v);
    if (v.trim() !== "" && percentIsValid(n)) {
      saveSettings({ uploadSizePercent: n });
    }
  }

  function handleLocalModeChange(m: SizeMode) {
    setLocalMode(m);
    if (m === "custom") {
      setLocalCustom(settings?.localSizePercent != null ? String(settings.localSizePercent) : "");
      return;
    }
    saveSettings({ localSizePercent: m === "full" ? null : Number(m) });
  }

  function handleLocalCustomChange(v: string) {
    setLocalCustom(v);
    const n = Number(v);
    if (v.trim() !== "" && percentIsValid(n)) {
      saveSettings({ localSizePercent: n });
    }
  }

  async function doSyncNow() {
    setSyncingNow(true);
    try {
      await syncNow();
    } catch (e) {
      console.error("Sync now failed:", e);
    } finally {
      setSyncingNow(false);
    }
  }

  async function doDisconnect() {
    if (!confirm("Disconnect from cloud storage? Sync will stop until you reconnect.")) return;
    setDisconnecting(true);
    try {
      await storageDisconnect();
      refreshStatus();
    } catch (e) {
      console.error("Disconnect failed:", e);
    } finally {
      setDisconnecting(false);
    }
  }

  return (
    <>
      <div className="settings-section">
        <span className="settings-label">Cloud sync</span>
        {status === undefined ? (
          <p className="settings-hint">Loading…</p>
        ) : status?.provider ? (
          <div className="storage-connected">
            <div className="storage-status-row">
              <span className={`storage-dot storage-dot-${status.state}`} />
              <span className="storage-provider-label">{providerLabel(status.provider)}</span>
            </div>
            {status.message && <p className="settings-hint">{status.message}</p>}
            <p className="settings-hint">
              {status.syncedCount} clip{status.syncedCount === 1 ? "" : "s"} synced, {status.pendingCount} pending
            </p>
            <div className="settings-actions">
              <button className="btn-sm" onClick={doSyncNow} disabled={syncingNow}>
                {syncingNow ? "Syncing…" : "Sync now"}
              </button>
              <button className="btn-cancel" onClick={doDisconnect} disabled={disconnecting}>
                {disconnecting ? "Disconnecting…" : "Disconnect"}
              </button>
            </div>
          </div>
        ) : (
          <StorageConnectForm onConnected={refreshStatus} />
        )}
      </div>

      <div className="settings-section">
        <span className="settings-label">Storage size</span>
        <SizeSelect
          label="Upload size"
          mode={uploadMode}
          custom={uploadCustom}
          onModeChange={handleUploadModeChange}
          onCustomChange={handleUploadCustomChange}
        />
        <SizeSelect
          label="New local recording size"
          mode={localMode}
          custom={localCustom}
          onModeChange={handleLocalModeChange}
          onCustomChange={handleLocalCustomChange}
          warning="Shrinking permanently re-encodes new recordings on this PC."
        />
        {settingsMsg && <div className="msg-error">{settingsMsg}</div>}
      </div>

      <div className="settings-section">
        <label className="music-loop">
          <input
            type="checkbox"
            checked={settings?.trayEnabled ?? false}
            onChange={e => saveSettings({ trayEnabled: e.target.checked })}
          />
          Keep syncing in the background when the window is closed
        </label>
        <div className="settings-actions">
          <button className="btn-sm" onClick={() => onOpenDiff?.()} disabled={!status?.provider}>
            Scan for differences
          </button>
        </div>
      </div>
    </>
  );
}
