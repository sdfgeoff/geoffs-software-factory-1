import ReactMarkdown from "react-markdown";
import rehypeMathjax from "rehype-mathjax";
import rehypeRaw from "rehype-raw";
import rehypeSanitize, {
  defaultSchema,
  type Options as SanitizeOptions,
} from "rehype-sanitize";
import remarkMath from "remark-math";

const svgTags = [
  "circle",
  "defs",
  "desc",
  "ellipse",
  "g",
  "line",
  "linearGradient",
  "path",
  "polygon",
  "polyline",
  "radialGradient",
  "rect",
  "stop",
  "svg",
  "text",
  "title",
  "tspan",
];
const svgAttributes = [
  "ariaLabel",
  "cx",
  "cy",
  "d",
  "dominantBaseline",
  "fill",
  "fillOpacity",
  "fillRule",
  "fontFamily",
  "fontSize",
  "fontWeight",
  "gradientTransform",
  "gradientUnits",
  "height",
  "offset",
  "opacity",
  "points",
  "preserveAspectRatio",
  "r",
  "role",
  "rx",
  "ry",
  "stopColor",
  "stopOpacity",
  "stroke",
  "strokeDasharray",
  "strokeLinecap",
  "strokeLinejoin",
  "strokeOpacity",
  "strokeWidth",
  "textAnchor",
  "transform",
  "viewBox",
  "width",
  "x",
  "x1",
  "x2",
  "xmlns",
  "y",
  "y1",
  "y2",
];
const messageSchema: SanitizeOptions = {
  ...defaultSchema,
  tagNames: [...(defaultSchema.tagNames ?? []), ...svgTags],
  attributes: {
    ...defaultSchema.attributes,
    code: [
      ...(defaultSchema.attributes?.code ?? []),
      ["className", "math-inline", "math-display"],
    ],
    ...Object.fromEntries(svgTags.map((tag) => [tag, svgAttributes])),
  },
};

export function MessageContent({ source }: { source: string }) {
  return (
    <div className="message-content">
      <ReactMarkdown
        remarkPlugins={[remarkMath]}
        rehypePlugins={[
          rehypeRaw,
          [rehypeSanitize, messageSchema],
          rehypeMathjax,
        ]}
      >
        {source}
      </ReactMarkdown>
    </div>
  );
}
