import { useEffect, useState, type FormEvent } from "react";
import { api } from "../api";
import type { BoardMessage, Channel } from "../types";
interface Props {
  channels: Channel[];
  refresh: number;
  onChanged: () => Promise<void>;
  onError: (error: unknown) => void;
}

export function MessageBoard({ channels, refresh, onChanged, onError }: Props) {
  const [channel, setChannel] = useState("general");
  const [messages, setMessages] = useState<BoardMessage[]>([]);
  const [body, setBody] = useState("");
  const [newChannel, setNewChannel] = useState("");
  useEffect(() => {
    void api.messages(channel).then(setMessages).catch(onError);
  }, [channel, refresh, onError]);
  async function post(event: FormEvent) {
    event.preventDefault();
    const destination = newChannel || channel;
    try {
      await api.postMessage(destination, body);
      setBody("");
      setNewChannel("");
      setChannel(destination);
      await onChanged();
    } catch (error) {
      onError(error);
    }
  }
  return (
    <section className="messages-layout">
      <aside>
        <h2>Channels</h2>
        {channels.map((item) => (
          <button
            className={channel === item.name ? "selected" : "ghost"}
            key={item.name}
            onClick={() => setChannel(item.name)}
          >
            #{item.name}
          </button>
        ))}
      </aside>
      <div className="conversation">
        <h2>#{channel}</h2>
        <div className="message-list">
          {messages.map((message) => (
            <article key={message.id}>
              <strong>{message.author}</strong>
              <time>
                {new Date(message.created_at * 1000).toLocaleString()}
              </time>
              <p>{message.body}</p>
            </article>
          ))}
        </div>
        <form onSubmit={post}>
          <label>
            New channel (optional)
            <input
              value={newChannel}
              onChange={(event) => setNewChannel(event.target.value)}
              placeholder={channel}
            />
          </label>
          <label>
            Message
            <textarea
              value={body}
              onChange={(event) => setBody(event.target.value)}
              required
            />
          </label>
          <button type="submit">Post</button>
        </form>
      </div>
    </section>
  );
}
