import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import { BoardSettings } from "./BoardSettings";

vi.mock("../api", () => ({
  api: { setSettings: vi.fn() },
}));

describe("BoardSettings", () => {
  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  it("selects or enters a channel and clears it to disable notifications", async () => {
    vi.mocked(api.setSettings).mockResolvedValue({
      merge_notification_channel: "release-notes",
    });
    const onChanged = vi.fn().mockResolvedValue(undefined);
    render(
      <BoardSettings
        settings={{ merge_notification_channel: "merges" }}
        channels={[
          { name: "general", description: "", most_recent_message_at: null },
          { name: "merges", description: "", most_recent_message_at: null },
        ]}
        onChanged={onChanged}
        onError={vi.fn()}
      />,
    );
    const input = screen.getByRole("combobox", {
      name: "Merge notification channel",
    });

    fireEvent.change(input, { target: { value: "not a channel" } });
    expect(screen.getByRole("alert").textContent).toContain("1–64 ASCII");
    expect(
      screen.getByRole("button", { name: "Save settings" }).hasAttribute("disabled"),
    ).toBe(true);

    fireEvent.change(input, { target: { value: "release-notes" } });
    fireEvent.submit(input.closest("form")!);
    await waitFor(() =>
      expect(api.setSettings).toHaveBeenCalledWith("release-notes"),
    );

    fireEvent.change(input, { target: { value: "" } });
    fireEvent.submit(input.closest("form")!);
    await waitFor(() => expect(api.setSettings).toHaveBeenLastCalledWith(null));
    expect(onChanged).toHaveBeenCalledTimes(2);
  });
});
