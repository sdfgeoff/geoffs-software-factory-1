import { useEffect, useState, type FormEvent } from "react";
import { api } from "../api";
import {
  channelDescriptionMaxLength,
  channelDescriptionRules,
  channelNameInputPattern,
  channelNamePattern,
  channelNameRules,
} from "../channelName";
import type { Agent, BoardMessage, Channel } from "../types";
import { MessageContent } from "./MessageContent";

interface Props {
  agents: Agent[];
  channels: Channel[];
  selectedChannelName?: string;
  refresh: number;
  liveMessages?: BoardMessage[];
  onSelectChannel?: (name: string) => void;
  onSelectAgent: (id: number) => void;
  onChanged: (channel?: Channel) => void | Promise<void>;
  onError: (error: unknown) => void;
}

export function MessageBoard({
  agents,
  channels,
  selectedChannelName,
  refresh,
  liveMessages = [],
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
  const [creatingChannel, setCreatingChannel] = useState(false);
  const [newChannel, setNewChannel] = useState("");
  const [newDescription, setNewDescription] = useState("");
  const validNewChannel = channelNamePattern.test(newChannel);
  const validDescription =
    [...newDescription].length <= channelDescriptionMaxLength;
  const selectedChannel = channels.find((item) => item.name === channel);
  const body = drafts[channel] ?? "";
  const visibleMessages = [
    ...new Map(
      [
        ...(messagesChannel === channel ? messages : []),
        ...liveMessages.filter((message) => message.channel_name === channel),
      ].map((message) => [message.id, message]),
    ).values(),
  ];
  useEffect(() => {
    if (selectedChannelName) setChannel(selectedChannelName);
  }, [selectedChannelName]);
  useEffect(() => {
    let current = true;
    setMessages([]);
    setLoading(true);
    void api
      .messages(channel)
      .then((next) => {
        if (current) {
          setMessages(next);
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
  async function post(event: FormEvent) {
    event.preventDefault();
    try {
      const created = await api.postMessage(channel, body);
      setMessages((current) => [...current, created]);
      setDrafts((current) => ({ ...current, [channel]: "" }));
    } catch (error) {
      onError(error);
    }
  }
  async function createChannel(event: FormEvent) {
    event.preventDefault();
    if (!validNewChannel || !validDescription) return;
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
      onError(error);
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
      <aside>
        <h2>Channels</h2>
        {creatingChannel ? (
          <form className="channel-setting" onSubmit={createChannel}>
            <label>
              Name
              <input
                autoFocus
                value={newChannel}
                onChange={(event) => setNewChannel(event.target.value)}
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
                onChange={(event) => setNewDescription(event.target.value)}
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
                disabled={!validNewChannel || !validDescription}
              >
                Save
              </button>
              <button
                type="button"
                className="ghost"
                onClick={() => {
                  setCreatingChannel(false);
                  setNewChannel("");
                  setNewDescription("");
                }}
              >
                Cancel
              </button>
            </div>
          </form>
        ) : (
          <button
            type="button"
            className="ghost"
            onClick={() => setCreatingChannel(true)}
          >
            + New channel
          </button>
        )}
        {channels.map((item) => (
          <button
            className={channel === item.name ? "selected" : "ghost"}
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
        <div className="message-list">
          {(loading || messagesChannel !== channel) && (
            <p className="empty">Loading messages…</p>
          )}
          {!loading && messagesChannel === channel && visibleMessages.length === 0 && (
            <p className="empty">No messages in #{channel} yet.</p>
          )}
          {visibleMessages.map((message) => {
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
                <time>
                  {new Date(message.created_at * 1000).toLocaleString()}
                </time>
                <MessageContent source={message.body} />
              </article>
            );
          })}
        </div>
        <form onSubmit={post}>
          <label>
            Message
            <textarea
              value={body}
              onChange={(event) =>
                setDrafts((current) => ({
                  ...current,
                  [channel]: event.target.value,
                }))
              }
              required
            />
          </label>
          <button type="submit">Post to #{channel}</button>
        </form>
      </div>
    </section>
  );
}
