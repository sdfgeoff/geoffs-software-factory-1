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

  it("prevents duplicate saves and keeps a failed value ready to retry", async () => {
    let rejectSave: (error: Error) => void = () => {};
    vi.mocked(api.setSettings).mockReturnValueOnce(
      new Promise((_, reject) => {
        rejectSave = reject;
      }),
    );
    const onError = vi.fn();
    render(
      <BoardSettings
        settings={{ merge_notification_channel: "merges" }}
        channels={[]}
        onChanged={async () => {}}
        onError={onError}
      />,
    );
    const input = screen.getByRole("combobox", {
      name: "Merge notification channel",
    });
    fireEvent.change(input, { target: { value: "release-notes" } });
    fireEvent.submit(input.closest("form")!);
    const saving = screen.getByRole("button", { name: "Saving…" });
    expect((saving as HTMLButtonElement).disabled).toBe(true);
    expect((input as HTMLInputElement).disabled).toBe(true);
    fireEvent.click(saving);
    expect(api.setSettings).toHaveBeenCalledTimes(1);

    rejectSave(new Error("Network unavailable"));
    expect(await screen.findByText("Could not save: Network unavailable")).toBeTruthy();
    expect((input as HTMLInputElement).value).toBe("release-notes");
    expect(onError).not.toHaveBeenCalled();

    vi.mocked(api.setSettings).mockResolvedValueOnce({
      merge_notification_channel: "release-notes",
    });
    fireEvent.submit(input.closest("form")!);
    expect(await screen.findByText("Saved")).toBeTruthy();
  });
});
