import { memo, useCallback, useEffect, useMemo, useRef, useState, type FormEvent, type RefObject } from "react";
import { api } from "../api";
import {
  channelDescriptionMaxLength,
  channelDescriptionRules,
  channelNameInputPattern,
  channelNamePattern,
  channelNameRules,
} from "../channelName";
import type { Agent, BoardMessage, Channel } from "../types";
import { mergeMessages } from "../messagePages";
import { MessageContent } from "./MessageContent";

interface Props {
  agents: Agent[];
  channels: Channel[];
  selectedChannelName?: string;
  refresh: number;
  catchUp?: number;
  liveMessages?: BoardMessage[];
  onSelectChannel?: (name: string) => void;
  onSelectAgent: (id: number) => void;
  onChanged: (channel?: Channel) => void | Promise<void>;
  onError: (error: unknown) => void;
}

const emptyMessages: BoardMessage[] = [];

const BoardConversation = memo(function BoardConversation({
  agents,
  channel,
  loading,
  messages,
  messagesChannel,
  hasOlder,
  loadingOlder,
  listRef,
  onLoadOlder,
  onSelectAgent,
}: {
  agents: Agent[];
  channel: string;
  loading: boolean;
  messages: BoardMessage[];
  messagesChannel: string | null;
  hasOlder: boolean;
  loadingOlder: boolean;
  listRef: RefObject<HTMLDivElement | null>;
  onLoadOlder: () => void;
  onSelectAgent: (id: number) => void;
}) {
  return (
    <div className="message-list" ref={listRef}>
      {(loading || messagesChannel !== channel) && (
        <p className="empty">Loading messages…</p>
      )}
      {!loading && messagesChannel === channel && messages.length === 0 && (
        <p className="empty">No messages in #{channel} yet.</p>
      )}
      {!loading && messagesChannel === channel && hasOlder && (
        <button type="button" className="ghost" disabled={loadingOlder} onClick={onLoadOlder}>
          {loadingOlder ? "Loading…" : "Load older messages"}
        </button>
      )}
      {messages.map((message) => {
        const agent = agents.find(
          (item) =>
            item.id === message.agent_id ||
            (message.agent_id === null && item.name === message.author),
        );
        return (
          <article key={message.id}>
            {agent ? (
              <button
                className="message-author"
                type="button"
                onClick={() => onSelectAgent(agent.id)}
              >
                {message.author}
              </button>
            ) : (
              <strong>{message.author}</strong>
            )}
            <time>{new Date(message.created_at * 1000).toLocaleString()}</time>
            <MessageContent source={message.body} />
          </article>
        );
      })}
    </div>
  );
});

export const MessageBoard = memo(function MessageBoard({
  agents,
  channels,
  selectedChannelName,
  refresh,
  catchUp = 0,
  liveMessages = emptyMessages,
  onSelectChannel,
  onSelectAgent,
  onChanged,
  onError,
}: Props) {
  const [channel, setChannel] = useState(selectedChannelName ?? "general");
  const [messages, setMessages] = useState<BoardMessage[]>([]);
  const [messagesChannel, setMessagesChannel] = useState<string | null>(null);
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState(true);
  const [hasOlder, setHasOlder] = useState(false);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const listRef = useRef<HTMLDivElement>(null);
  const lastCatchUp = useRef(catchUp);
  const [creatingChannel, setCreatingChannel] = useState(false);
  const [newChannel, setNewChannel] = useState("");
  const [newDescription, setNewDescription] = useState("");
  const [postingChannel, setPostingChannel] = useState<string | null>(null);
  const [postStatuses, setPostStatuses] = useState<Record<string, string>>({});
  const [channelSaving, setChannelSaving] = useState(false);
  const [channelStatus, setChannelStatus] = useState("");
  const validNewChannel = channelNamePattern.test(newChannel);
  const validDescription =
    [...newDescription].length <= channelDescriptionMaxLength;
  const selectedChannel = channels.find((item) => item.name === channel);
  const body = drafts[channel] ?? "";
  const visibleMessages = useMemo(
    () => mergeMessages(
      messagesChannel === channel ? messages : [],
      liveMessages.filter((message) => message.channel_name === channel),
    ),
    [channel, liveMessages, messages, messagesChannel],
  );
  useEffect(() => {
    if (selectedChannelName) setChannel(selectedChannelName);
  }, [selectedChannelName]);
  useEffect(() => {
    let current = true;
    lastCatchUp.current = catchUp;
    setMessages([]);
    setLoading(true);
    void api
      .messages(channel, { limit: 50 })
      .then((page) => {
        if (current) {
          setMessages(page.messages);
          setHasOlder(page.has_more);
          setMessagesChannel(channel);
        }
      })
      .catch(onError)
      .finally(() => {
        if (current) setLoading(false);
      });
    return () => {
      current = false;
    };
  }, [channel, refresh, onError]);
  useEffect(() => {
    if (catchUp < 0) {
      lastCatchUp.current = catchUp;
      return;
    }
    if (catchUp === lastCatchUp.current || messagesChannel !== channel) return;
    lastCatchUp.current = catchUp;
    let current = true;
    void (async () => {
      let cursor = visibleMessages.at(-1)?.id;
      if (!cursor) {
        const page = await api.messages(channel, { limit: 50 });
        if (current) {
          setMessages(page.messages);
          setHasOlder(page.has_more);
        }
        return;
      }
      while (current) {
        const page = await api.messages(channel, { after_id: cursor, limit: 100 });
        if (!current) return;
        setMessages((existing) => mergeMessages(existing, page.messages));
        const nextCursor = page.messages.at(-1)?.id;
        if (!page.has_more || !nextCursor) return;
        cursor = nextCursor;
      }
    })().catch(onError);
    return () => {
      current = false;
    };
  }, [catchUp, channel, messagesChannel, onError]);
  const loadOlder = useCallback(async () => {
    const beforeId = visibleMessages[0]?.id;
    if (!beforeId || loadingOlder) return;
    const list = listRef.current;
    const previousHeight = list?.scrollHeight ?? 0;
    setLoadingOlder(true);
    try {
      const page = await api.messages(channel, { before_id: beforeId, limit: 50 });
      setMessages((current) => mergeMessages(page.messages, current));
      setHasOlder(page.has_more);
      window.requestAnimationFrame(() => {
        if (list) list.scrollTop += list.scrollHeight - previousHeight;
      });
    } catch (error) {
      onError(error);
    } finally {
      setLoadingOlder(false);
    }
  }, [channel, loadingOlder, onError, visibleMessages]);
  async function post(event: FormEvent) {
    event.preventDefault();
    if (postingChannel !== null) return;
    const targetChannel = channel;
    setPostingChannel(targetChannel);
    setPostStatuses((current) => ({ ...current, [targetChannel]: "Posting…" }));
    try {
      const created = await api.postMessage(targetChannel, body);
      setMessages((current) => mergeMessages(current, [created]));
      setDrafts((current) => ({ ...current, [targetChannel]: "" }));
      setPostStatuses((current) => ({ ...current, [targetChannel]: "Posted" }));
    } catch (error) {
      setPostStatuses((current) => ({
        ...current,
        [targetChannel]: `Could not post: ${error instanceof Error ? error.message : "Unknown error"}`,
      }));
    } finally {
      setPostingChannel((current) => current === targetChannel ? null : current);
    }
  }
  async function createChannel(event: FormEvent) {
    event.preventDefault();
    if (!validNewChannel || !validDescription || channelSaving) return;
    setChannelSaving(true);
    setChannelStatus("Creating channel…");
    try {
      const created = await api.createChannel(newChannel, newDescription);
      setChannel(created.name);
      onSelectChannel?.(created.name);
      setMessages([]);
      setCreatingChannel(false);
      setNewChannel("");
      setNewDescription("");
      await onChanged(created);
    } catch (error) {
      setChannelStatus(
        `Could not create channel: ${error instanceof Error ? error.message : "Unknown error"}`,
      );
    } finally {
      setChannelSaving(false);
    }
  }
  function selectChannel(name: string) {
    setChannel(name);
    onSelectChannel?.(name);
    setCreatingChannel(false);
    setNewChannel("");
    setNewDescription("");
  }
  return (
    <section className="messages-layout">
      <aside className="context-sidebar">
        <h2>Channels</h2>
        <label className="mobile-context-picker">
          Channel
          <select value={channel} onChange={(event) => selectChannel(event.target.value)}>
            {channels.map((item) => (
              <option key={item.name} value={item.name}>#{item.name}</option>
            ))}
          </select>
        </label>
        {creatingChannel ? (
          <form className="channel-setting" onSubmit={createChannel}>
            <label>
              Name
              <input
                autoFocus
                value={newChannel}
                disabled={channelSaving}
                onChange={(event) => {
                  setNewChannel(event.target.value);
                  setChannelStatus("");
                }}
                pattern={channelNameInputPattern}
                maxLength={64}
                aria-describedby="channel-name-rules"
                required
              />
            </label>
            <small id="channel-name-rules">{channelNameRules}</small>
            {newChannel && !validNewChannel && (
              <p className="inline-error" role="alert">
                {channelNameRules}
              </p>
            )}
            <label>
              Description
              <textarea
                value={newDescription}
                disabled={channelSaving}
                onChange={(event) => {
                  setNewDescription(event.target.value);
                  setChannelStatus("");
                }}
                maxLength={channelDescriptionMaxLength}
                aria-describedby="channel-description-rules"
              />
            </label>
            <small id="channel-description-rules">
              {channelDescriptionRules}
            </small>
            {!validDescription && (
              <p className="inline-error" role="alert">
                {channelDescriptionRules}
              </p>
            )}
            <div className="field-row">
              <button
                type="submit"
                disabled={!validNewChannel || !validDescription || channelSaving}
              >
                {channelSaving ? "Creating…" : "Save"}
              </button>
              <button
                type="button"
                className="ghost"
                disabled={channelSaving}
                onClick={() => {
                  setCreatingChannel(false);
                  setNewChannel("");
                  setNewDescription("");
                }}
              >
                Cancel
              </button>
            </div>
            {channelStatus && (
              <p
                className={channelStatus.startsWith("Could not") ? "inline-error" : undefined}
                role="status"
                aria-live="polite"
              >
                {channelStatus}
              </p>
            )}
          </form>
        ) : (
          <button
            type="button"
            className="ghost context-create-button"
            onClick={() => setCreatingChannel(true)}
          >
            + New channel
          </button>
        )}
        {channels.map((item) => (
          <button
            className={`${channel === item.name ? "selected" : "ghost"} context-option`}
            key={item.name}
            aria-current={channel === item.name ? "page" : undefined}
            onClick={() => selectChannel(item.name)}
          >
            #{item.name}
          </button>
        ))}
      </aside>
      <div className="conversation">
        <h2>#{channel}</h2>
        {selectedChannel?.description && (
          <p className="channel-description">{selectedChannel.description}</p>
        )}
        <BoardConversation
          agents={agents}
          channel={channel}
          loading={loading}
          messages={visibleMessages}
          messagesChannel={messagesChannel}
          hasOlder={hasOlder}
          loadingOlder={loadingOlder}
          listRef={listRef}
          onLoadOlder={loadOlder}
          onSelectAgent={onSelectAgent}
        />
        <form onSubmit={post}>
          <label>
            Message
            <textarea
              value={body}
              disabled={postingChannel !== null}
              onChange={(event) =>
                {
                  setDrafts((current) => ({
                    ...current,
                    [channel]: event.target.value,
                  }));
                  setPostStatuses((current) => ({ ...current, [channel]: "" }));
                }
              }
              required
            />
          </label>
          <button type="submit" disabled={postingChannel !== null}>
            {postingChannel === channel ? "Posting…" : `Post to #${channel}`}
          </button>
          {postStatuses[channel] && (
            <p
              className={postStatuses[channel].startsWith("Could not") ? "inline-error" : undefined}
              role="status"
              aria-live="polite"
            >
              {postStatuses[channel]}
            </p>
          )}
        </form>
      </div>
    </section>
  );
});
