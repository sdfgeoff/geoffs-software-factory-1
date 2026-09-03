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

type HastNode = {
  tagName?: string;
  properties?: Record<string, unknown>;
  children?: HastNode[];
};

function namespaceSvgFragmentReferences() {
  const prefix = messageSchema.clobberPrefix ?? "user-content-";

  return (tree: HastNode) => {
    function visit(node: HastNode, insideSvg = false) {
      const isSvg = insideSvg || node.tagName === "svg";

      if (isSvg && node.properties) {
        for (const [name, value] of Object.entries(node.properties)) {
          if (typeof value === "string") {
            node.properties[name] = value.replace(
              /url\(\s*#([^\s)]+)\s*\)/g,
              `url(#${prefix}$1)`,
            );
          }
        }
      }

      node.children?.forEach((child) => visit(child, isSvg));
    }

    visit(tree);
  };
}

export function MessageContent({ source }: { source: string }) {
  return (
    <div className="message-content">
      <ReactMarkdown
        remarkPlugins={[remarkMath]}
        rehypePlugins={[
          rehypeRaw,
          [rehypeSanitize, messageSchema],
          namespaceSvgFragmentReferences,
          rehypeMathjax,
        ]}
      >
        {source}
      </ReactMarkdown>
    </div>
  );
}
