import { createSignal, type Accessor, type JSXElement } from "solid-js";
import Context from "./SheetContextSolid";
import "./Sheet.scss";

export function Root({ children }: { children: JSXElement }) {
  const [isOpen, setIsOpen] = createSignal(false);
  const [isTriggerHover, setTriggerHover] = createSignal(false);

  function handleEsc(e: KeyboardEvent) {
    if (e.key === "Escape") {
      toggle();
    }
  }

  function toggle() {
    const oldState = isOpen();
    // TODO change focus to the first focusable element in the sheet for
    // accessibility
    if (!oldState) {
      document.addEventListener("keydown", handleEsc);
      document.body.classList.add("noscroll");
    } else {
      document.removeEventListener("keydown", handleEsc);
      document.body.classList.remove("noscroll");
    }
    setIsOpen(!oldState);
  }

  const { SetSheetContext } = Context;

  SetSheetContext((context) => {
    return {
      isOpen: isOpen,
      isTriggerHover: isTriggerHover,
      setTriggerHover: setTriggerHover,
      toggle: toggle,
      initialized: true,
    };
  });

  return (
    <div class={`side-sheet${isOpen() ? " open" : ""}`}>
      <div class="sheet-overlay" onClick={toggle}></div>
      <div class="sheet-content">{children}</div>
    </div>
  );
}

export function Trigger({
  children,
  ...rest
}: {
  children: JSXElement;
  class?: string;
}) {
  function toggle() {
    const { SheetContext } = Context;
    const c = SheetContext();
    c.toggle();
  }

  return (
    <button
      {...rest}
      onClick={toggle}
      onMouseOver={() => {
        const { SheetContext } = Context;
        const c = SheetContext();
        c.setTriggerHover(true);
      }}
      onMouseLeave={() => {
        const { SheetContext } = Context;
        const c = SheetContext();
        c.setTriggerHover(false);
      }}
    >
      {children}
    </button>
  );
}

export default {
  Root: Root,
  Trigger,
};
