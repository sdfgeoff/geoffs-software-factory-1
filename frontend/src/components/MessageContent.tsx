import { memo } from "react";
import ReactMarkdown from "react-markdown";
import rehypeKatex from "rehype-katex";
import rehypeRaw from "rehype-raw";
import rehypeSanitize, {
  defaultSchema,
  type Options as SanitizeOptions,
} from "rehype-sanitize";
import remarkMath from "remark-math";
import "katex/dist/katex.min.css";

const messageSchema: SanitizeOptions = {
  ...defaultSchema,
  attributes: {
    ...defaultSchema.attributes,
    code: [
      ...(defaultSchema.attributes?.code ?? []),
      ["className", "math-inline", "math-display"],
    ],
  },
};

export const MessageContent = memo(function MessageContent({ source }: { source: string }) {
  return (
    <div className="message-content">
      <ReactMarkdown
        remarkPlugins={[remarkMath]}
        rehypePlugins={[
          rehypeRaw,
          [rehypeSanitize, messageSchema],
          rehypeKatex,
        ]}
      >
        {source}
      </ReactMarkdown>
    </div>
  );
});
