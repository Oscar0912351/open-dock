import { act, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import "../../../shared/theme/tokens.css";
import "../../WidgetShell.css";
import { Clock, defaultClockSettings } from "./Clock";
import { RollingNumber } from "./RollingNumber";

const now = new Date(2026, 10, 16, 14, 32, 0);

describe("Ur", () => {
  it.each(["pill", "s", "m"] as const)("renderer størrelsen %s", (size) => {
    const { container } = render(
      <Clock now={now} settings={defaultClockSettings} size={size} theme="aurora" />,
    );
    expect(screen.getByRole("group", { name: /Ur: 14:32/ })).toBeTruthy();
    expect(container.querySelector(".clock")).toBeTruthy();
  });

  it("renderer analog visning i lille størrelse", () => {
    const { container } = render(
      <Clock
        now={now}
        settings={{ ...defaultClockSettings, style: "analog", showSeconds: true }}
        size="s"
        theme="laerk"
      />,
    );
    expect(container.querySelectorAll(".clock-tick")).toHaveLength(12);
    expect(container.querySelector(".clock-second")).toBeTruthy();
  });

  it("viser ugens dage når mellemuret ikke har ekstra byer", () => {
    render(<Clock now={now} settings={defaultClockSettings} size="m" theme="aurora" />);
    expect(document.querySelectorAll(".clock-week-day")).toHaveLength(7);
    expect(document.querySelector(".clock-week-day.is-today")?.textContent).toBe("M");
  });

  it("ruller ændrede cifre og beholder uændrede cifre stille", () => {
    vi.useFakeTimers();
    const { rerender } = render(<RollingNumber value="14:32" />);
    rerender(<RollingNumber value="14:33" />);
    expect(document.querySelectorAll(".rolling-digit")).toHaveLength(1);
    act(() => vi.advanceTimersByTime(340));
    expect(document.querySelectorAll(".rolling-digit")).toHaveLength(0);
    vi.useRealTimers();
  });
});
