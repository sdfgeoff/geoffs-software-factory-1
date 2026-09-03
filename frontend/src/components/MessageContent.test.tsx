import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { MessageContent } from "./MessageContent";

describe("MessageContent", () => {
  it("renders rich message formats while removing unsafe HTML", () => {
    const { container } = render(
      <MessageContent
        source={`**Markdown**

<section><em>HTML</em><script>alert("unsafe")</script></section>

<svg viewBox="0 0 10 10"><circle cx="5" cy="5" r="4" fill="red" onload="alert('unsafe')" /></svg>

$E = mc^2$`}
      />,
    );

    expect(screen.getByText("Markdown").tagName).toBe("STRONG");
    expect(screen.getByText("HTML").closest("section")).toBeTruthy();
    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("circle")?.getAttribute("onload")).toBeNull();
    expect(container.querySelector("svg circle")).toBeTruthy();
    expect(container.querySelector("mjx-container svg")).toBeTruthy();
  });

  it("keeps sanitized SVG fragment references linked to their definitions", () => {
    const { container } = render(
      <MessageContent
        source={`<svg viewBox="0 0 10 10"><defs><linearGradient id="fade"><stop offset="0%" stop-color="red" /><stop offset="100%" stop-color="blue" /></linearGradient></defs><rect width="10" height="10" fill="url(#fade)" /></svg>`}
      />,
    );

    expect(container.querySelector("linearGradient")?.id).toBe(
      "user-content-fade",
    );
    expect(container.querySelector("rect")?.getAttribute("fill")).toBe(
      "url(#user-content-fade)",
    );
  });
});
