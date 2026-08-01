/** @vitest-environment jsdom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  injectStdin: vi.fn(async () => {}),
}));

vi.mock("../lib/api", () => ({
  api: {
    session: {
      injectStdin: mocks.injectStdin,
    },
  },
}));

import { InboxBlockedPill } from "./InboxBlockedPill";

describe("InboxBlockedPill", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    mocks.injectStdin.mockClear();
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.unstubAllGlobals();
  });

  async function render(
    props: Partial<React.ComponentProps<typeof InboxBlockedPill>> = {},
  ) {
    await act(async () => {
      root.render(
        <InboxBlockedPill
          sessionId="S-IMPL"
          unreadCount={2}
          narrow={false}
          onError={() => {}}
          {...props}
        />,
      );
    });
  }

  it("renders the draft copy and clears it with Ctrl-U", async () => {
    await render();

    expect(container.textContent).toContain("Inbox waiting (2)");
    expect(container.textContent).toContain(
      "— delivery paused: you have unsent input here",
    );
    expect(container.textContent).toContain("Clear draft");
    expect(container.textContent).toContain("⌃U");

    await act(async () => {
      container
        .querySelector("button")
        ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(mocks.injectStdin).toHaveBeenCalledTimes(1);
    expect(mocks.injectStdin).toHaveBeenCalledWith("S-IMPL", "\x15");
    expect(mocks.injectStdin).not.toHaveBeenCalledWith("S-IMPL", "\r");
    expect(mocks.injectStdin).not.toHaveBeenCalledWith("S-IMPL", "\x03");
  });

  it("omits the count for one unread message", async () => {
    await render({ unreadCount: 1 });

    expect(container.textContent).toContain("Inbox waiting");
    expect(container.textContent).not.toContain("Inbox waiting (1)");
  });

  it("hides the body in a narrow pane", async () => {
    await render({ narrow: true });

    expect(container.textContent).toContain("Inbox waiting (2)");
    expect(container.textContent).not.toContain(
      "— delivery paused: you have unsent input here",
    );
    expect(container.textContent).toContain("Clear draft");
    expect(container.querySelector("button")).not.toBeNull();
  });
});
