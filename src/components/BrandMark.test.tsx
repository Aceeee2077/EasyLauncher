import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { BrandMark } from "./BrandMark";

describe("BrandMark", () => {
  it("scales to the requested size and stays decorative by default", () => {
    const { container } = render(<BrandMark size={48} />);
    const svg = container.querySelector("svg");
    expect(svg).toHaveAttribute("width", "48");
    expect(svg).toHaveAttribute("height", "48");
    expect(svg).toHaveAttribute("aria-hidden", "true");
    expect(svg).not.toHaveAttribute("role");
  });

  it("exposes an accessible name when a title is provided", () => {
    render(<BrandMark title="EasyLauncher" />);
    expect(screen.getByRole("img", { name: "EasyLauncher" })).toBeVisible();
  });

  it("draws one filled silhouette plus a translucent sun", () => {
    const { container } = render(<BrandMark />);
    const shapes = container.querySelectorAll("path, circle");
    expect(shapes).toHaveLength(2);
    expect(shapes[0].tagName).toBe("circle");
    expect(shapes[0]).toHaveAttribute("fill", "currentColor");
    expect(shapes[0]).toHaveAttribute("opacity", "0.55");
    expect(shapes[1].tagName).toBe("path");
    expect(shapes[1]).toHaveAttribute("fill", "currentColor");
  });
});
