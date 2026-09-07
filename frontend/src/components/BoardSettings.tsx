import { useEffect, useState, type FormEvent } from "react";
import { api } from "../api";
import {
  channelNameInputPattern,
  channelNamePattern,
  channelNameRules,
} from "../channelName";
import type { BoardSettings as Settings, Channel } from "../types";

interface Props {
  settings: Settings;
  channels: Channel[];
  onChanged: (settings: Settings) => void | Promise<void>;
  onError: (error: unknown) => void;
}

export function BoardSettings({
  settings,
  channels,
  onChanged,
}: Props) {
  const [channel, setChannel] = useState(
    settings.merge_notification_channel ?? "",
  );
  const [saving, setSaving] = useState(false);
  const [status, setStatus] = useState("");
  const valid = channel === "" || channelNamePattern.test(channel);

  useEffect(
    () => setChannel(settings.merge_notification_channel ?? ""),
    [settings.merge_notification_channel],
  );

  async function save(event: FormEvent) {
    event.preventDefault();
    if (!valid || saving) return;
    setSaving(true);
    setStatus("Saving…");
    try {
      await onChanged(await api.setSettings(channel || null));
      setStatus("Saved");
    } catch (error) {
      setStatus(
        `Could not save: ${error instanceof Error ? error.message : "Unknown error"}`,
      );
    } finally {
      setSaving(false);
    }
  }

  return (
    <section className="board-settings">
      <h2>Board settings</h2>
      <form onSubmit={save}>
        <label htmlFor="merge-notification-channel">
          Merge notification channel
        </label>
        <input
          id="merge-notification-channel"
          list="merge-notification-channels"
          value={channel}
          disabled={saving}
          onChange={(event) => {
            setChannel(event.target.value);
            setStatus("");
          }}
          pattern={channelNameInputPattern}
          maxLength={64}
          aria-describedby="merge-channel-help merge-channel-rules"
          placeholder="Disabled"
        />
        <datalist id="merge-notification-channels">
          {channels.map((item) => (
            <option key={item.name} value={item.name} />
          ))}
        </datalist>
        <small id="merge-channel-help">
          Successful merges are posted through the normal channel message and
          agent wake flow. Leave empty to disable notifications.
        </small>
        <small id="merge-channel-rules">{channelNameRules}</small>
        {channel && !valid && (
          <p className="inline-error" role="alert">
            {channelNameRules}
          </p>
        )}
        <button type="submit" disabled={!valid || saving}>
          {saving ? "Saving…" : "Save settings"}
        </button>
        <p
          className={status.startsWith("Could not") ? "inline-error" : undefined}
          role="status"
          aria-live="polite"
        >
          {status}
        </p>
      </form>
    </section>
  );
}
