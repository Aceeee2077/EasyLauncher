import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { LanguageProvider } from "../i18n";
import { Button, Empty, bytes } from "./ui";

describe("bytes", () => {
  it("formats each size tier", () => {
    expect(bytes(0)).toBe("0 B");
    expect(bytes(1023)).toBe("1023 B");
    expect(bytes(1024)).toBe("1.0 KB");
    expect(bytes(1024 ** 2)).toBe("1.0 MB");
    expect(bytes(1024 ** 3)).toBe("1.00 GB");
  });
});

describe("Button", () => {
  it("marks a busy button disabled and renders a spinner", () => {
    const { container } = render(<Button busy>Save</Button>);
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(container.querySelector(".spin")).toBeInTheDocument();
  });
});

describe("Empty", () => {
  it("renders title, description and action", () => {
    render(
      <LanguageProvider>
        <Empty
          title="No worlds yet"
          description="Create your first server."
          action={<button>Create</button>}
        />
      </LanguageProvider>,
    );
    expect(
      screen.getByRole("heading", { name: "No worlds yet" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Create your first server.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Create" })).toBeInTheDocument();
  });
});
