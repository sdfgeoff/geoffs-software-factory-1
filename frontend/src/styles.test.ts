// @vitest-environment node
// @ts-expect-error Vitest provides Node's built-ins for this stylesheet contract test.
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const styles = readFileSync(new URL("./styles.css", import.meta.url), "utf8");

describe("responsive layouts", () => {
  it("defines tablet and narrow-screen layouts for every workspace", () => {
    expect(styles).toContain("@media (max-width: 980px)");
    expect(styles).toContain("@media (max-width: 820px)");
    expect(styles).toContain(".kanban .column:not(.mobile-active)");
    expect(styles).toContain(".context-sidebar > .context-option");
    expect(styles).toContain(".agent-create-form.mobile-open");
  });
});
