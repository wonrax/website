import { createSignal, type JSXElement, splitProps } from "solid-js";
import Context from "./SheetContextSolid";
import "./Sheet.scss";

export function Root(props: { children: JSXElement }): JSXElement {
  const [isOpen, setIsOpen] = createSignal(false);
  const [isTriggerHover, setTriggerHover] = createSignal(false);

  function handleEsc(e: KeyboardEvent): void {
    if (e.key === "Escape") {
      toggle();
    }
  }

  function toggle(): void {
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

  SetSheetContext(() => {
    return {
      isOpen,
      isTriggerHover,
      setTriggerHover,
      toggle,
      initialized: true,
    };
  });

  return (
    <div class={`side-sheet${isOpen() ? " open" : ""}`}>
      <div class="sheet-overlay" onClick={toggle} />
      <div class="sheet-content">{props.children}</div>
    </div>
  );
}

export function Trigger(_props: {
  children: JSXElement;
  class?: string;
}): JSXElement {
  const [props, rest] = splitProps(_props, ["children"]);

  function toggle(): void {
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
      {props.children}
    </button>
  );
}

export default {
  Root,
  Trigger,
};
