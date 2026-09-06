import { useEffect, useState, type FormEvent } from "react";
import { api } from "../api";
import {
  channelNameInputPattern,
  channelNamePattern,
  channelNameRules,
} from "../channelName";
import type { Agent, BoardMessage, Channel } from "../types";
import { MessageContent } from "./MessageContent";

interface Props {
  agents: Agent[];
  channels: Channel[];
  refresh: number;
  onSelectAgent: (id: number) => void;
  onChanged: () => Promise<void>;
  onError: (error: unknown) => void;
}

export function MessageBoard({
  agents,
  channels,
  refresh,
  onSelectAgent,
  onChanged,
  onError,
}: Props) {
  const [channel, setChannel] = useState("general");
  const [messages, setMessages] = useState<BoardMessage[]>([]);
  const [body, setBody] = useState("");
  const [creatingChannel, setCreatingChannel] = useState(false);
  const [newChannel, setNewChannel] = useState("");
  const destination = creatingChannel ? newChannel : channel;
  const validDestination =
    !creatingChannel || channelNamePattern.test(newChannel);
  useEffect(() => {
    void api.messages(channel).then(setMessages).catch(onError);
  }, [channel, refresh, onError]);
  async function post(event: FormEvent) {
    event.preventDefault();
    if (!validDestination) return;
    try {
      await api.postMessage(destination, body);
      setBody("");
      setCreatingChannel(false);
      setNewChannel("");
      setChannel(destination);
      await onChanged();
    } catch (error) {
      onError(error);
    }
  }
  function selectChannel(name: string) {
    setChannel(name);
    setCreatingChannel(false);
    setNewChannel("");
  }
  return (
    <section className="messages-layout">
      <aside>
        <h2>Channels</h2>
        {creatingChannel ? (
          <div className="channel-setting">
            <label>
              Channel name
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
            {newChannel && !validDestination && (
              <p className="inline-error" role="alert">
                {channelNameRules}
              </p>
            )}
            <button
              type="button"
              className="ghost"
              onClick={() => {
                setCreatingChannel(false);
                setNewChannel("");
              }}
            >
              Cancel
            </button>
          </div>
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
            onClick={() => selectChannel(item.name)}
          >
            #{item.name}
          </button>
        ))}
      </aside>
      <div className="conversation">
        <h2>#{channel}</h2>
        <div className="message-list">
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
              onChange={(event) => setBody(event.target.value)}
              required
            />
          </label>
          <button type="submit" disabled={!validDestination}>
            Post to #{destination || "…"}
          </button>
        </form>
      </div>
    </section>
  );
}
