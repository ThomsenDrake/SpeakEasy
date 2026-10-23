import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { toast } from "./toast";

const { mockCustom, mockDismiss } = vi.hoisted(() => ({
  mockCustom: vi.fn(),
  mockDismiss: vi.fn(),
}));

vi.mock("sonner", () => ({
  toast: {
    custom: mockCustom,
    dismiss: mockDismiss,
  },
}));

vi.mock("lucide-react", () => ({
  BellDot: () => <span data-testid="icon-BellDot" />,
  CircleCheck: () => <span data-testid="icon-CircleCheck" />,
  LoaderCircle: () => <span data-testid="icon-LoaderCircle" />,
  TriangleAlert: () => <span data-testid="icon-TriangleAlert" />,
  X: () => <span data-testid="icon-X" />,
}));

function renderToast(id = "test-id") {
  const renderFn = mockCustom.mock.lastCall?.[0] as
    | ((id: string) => React.ReactElement)
    | undefined;
  if (!renderFn) throw new Error("toast was not called");
  return render(renderFn(id));
}

afterEach(cleanup);

describe("toast wrapper behavior", () => {
  beforeEach(() => {
    mockCustom.mockClear();
    mockDismiss.mockClear();
  });

  it("toast.dismiss delegates to sonnerToast.dismiss", () => {
    toast.dismiss("abc");
    expect(mockDismiss).toHaveBeenCalledWith("abc");
    expect(mockDismiss).toHaveBeenCalledTimes(1);
  });

  it("forwards toastOptions as second argument to sonner.custom", () => {
    toast("Hello", { duration: 5000, id: "my-id", toasterId: "main" });
    expect(mockCustom).toHaveBeenCalledWith(expect.any(Function), {
      duration: 5000,
      id: "my-id",
      toasterId: "main",
    });
  });

  it("passes undefined toastOptions when none provided (sonner default duration applies)", () => {
    toast("Hello");
    expect(mockCustom).toHaveBeenCalledWith(expect.any(Function), undefined);
  });

  it("converts a string to { title } and renders it", () => {
    toast.success("My title");
    renderToast();
    expect(screen.getByText("My title")).toBeInTheDocument();
  });

  it("explicit variant on object overrides intent", () => {
    toast({ title: "Test", variant: "error" });
    renderToast();
    expect(screen.getByTestId("icon-TriangleAlert")).toBeInTheDocument();
  });
});

describe("rendered output — success (full feature set)", () => {
  beforeEach(() => {
    mockCustom.mockClear();
    mockDismiss.mockClear();
  });

  describe("default rendering (no extras)", () => {
    it("renders title text and success icon", () => {
      toast.success("Success title");
      renderToast();
      expect(screen.getByText("Success title")).toBeInTheDocument();
      expect(screen.getByTestId("icon-CircleCheck")).toBeInTheDocument();
    });

    it("does not render action buttons", () => {
      toast.success("Success title");
      renderToast();
      expect(screen.queryAllByRole("button")).toHaveLength(0);
    });

    it("does not render dismiss button", () => {
      toast.success("Success title");
      renderToast();
      expect(
        screen.queryByTestId("toast-dismiss-button"),
      ).not.toBeInTheDocument();
    });
  });

  describe("with all options", () => {
    it("renders description when provided", () => {
      toast.success({ title: "Title", description: "A description" });
      renderToast();
      expect(screen.getByText("A description")).toBeInTheDocument();
    });

    it("renders one action button", () => {
      toast.success({
        title: "Title",
        actions: [{ children: "Confirm", onClick: vi.fn() }],
      });
      renderToast();
      expect(
        screen.getByRole("button", { name: "Confirm" }),
      ).toBeInTheDocument();
    });

    it("renders two action buttons", () => {
      toast.success({
        title: "Title",
        actions: [
          { children: "Confirm", onClick: vi.fn() },
          { children: "Cancel", onClick: vi.fn() },
        ],
      });
      renderToast();
      expect(
        screen.getByRole("button", { name: "Confirm" }),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: "Cancel" }),
      ).toBeInTheDocument();
    });

    it("clicking an action calls its onClick handler", () => {
      const mockAction = vi.fn();
      toast.success({
        title: "Title",
        actions: [{ children: "Do it", onClick: mockAction }],
      });
      renderToast();
      fireEvent.click(screen.getByRole("button", { name: "Do it" }));
      expect(mockAction).toHaveBeenCalledTimes(1);
    });

    it("clicking an action also dismisses the toast", () => {
      const toastId = "my-toast";
      toast.success({
        title: "Title",
        actions: [{ children: "Do it", onClick: vi.fn() }],
      });
      renderToast(toastId);
      fireEvent.click(screen.getByRole("button", { name: "Do it" }));
      expect(mockDismiss).toHaveBeenCalledWith(toastId);
    });

    it("renders dismiss button when dismissible: true", () => {
      toast.success("Title", { dismissible: true });
      renderToast();
      expect(screen.getByTestId("toast-dismiss-button")).toBeInTheDocument();
    });

    it("clicking dismiss button calls sonnerToast.dismiss with the toast id", () => {
      const toastId = "dismissible-toast";
      toast.success("Title", { dismissible: true });
      renderToast(toastId);
      const dismissButton = screen.getByTestId("toast-dismiss-button");
      expect(dismissButton).toBeInTheDocument();
      fireEvent.click(dismissButton);
      expect(mockDismiss).toHaveBeenCalledWith(toastId);
    });
  });
});

describe("variant rendering", () => {
  beforeEach(() => {
    mockCustom.mockClear();
    mockDismiss.mockClear();
  });

  it("toast() renders no icon (info intent)", () => {
    toast("Title");
    renderToast();
    expect(screen.queryByTestId(/^icon-/)).not.toBeInTheDocument();
  });

  it("toast.info() renders no icon", () => {
    toast.info("Title");
    renderToast();
    expect(screen.queryByTestId(/^icon-/)).not.toBeInTheDocument();
  });

  it("toast.success() renders CircleCheck icon", () => {
    toast.success("Title");
    renderToast();
    expect(screen.getByTestId("icon-CircleCheck")).toBeInTheDocument();
  });

  it("toast.error() renders TriangleAlert icon", () => {
    toast.error("Title");
    renderToast();
    expect(screen.getByTestId("icon-TriangleAlert")).toBeInTheDocument();
  });

  it("toast.warning() renders BellDot icon", () => {
    toast.warning("Title");
    renderToast();
    expect(screen.getByTestId("icon-BellDot")).toBeInTheDocument();
  });

  it("toast.loading() renders LoaderCircle icon", () => {
    toast.loading("Title");
    renderToast();
    expect(screen.getByTestId("icon-LoaderCircle")).toBeInTheDocument();
  });
});
