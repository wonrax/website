import {
  createSignal,
  onCleanup,
  type JSX,
  type JSXElement,
  splitProps,
} from "solid-js";
import Context from "./SheetContext";
import "./Sheet.scss";

export function Root(props: { children: JSXElement }): JSXElement {
  const [isOpen, setIsOpen] = createSignal(false);
  const [isSheetTriggerButtonHovered, setSheetTriggerButtonHovered] =
    createSignal(false);

  function handleEsc(e: KeyboardEvent): void {
    if (e.key === "Escape" && isOpen()) {
      close();
    }
  }

  function open(): void {
    if (typeof document === "undefined") {
      return;
    }

    if (!isOpen()) {
      document.addEventListener("keydown", handleEsc);
      document.body.classList.add("noscroll");
      setIsOpen(true);
      queueMicrotask(() => {
        document
          .querySelector<HTMLElement>(
            ".sheet-content [autofocus], .sheet-content button, .sheet-content a, .sheet-content input, .sheet-content textarea, .sheet-content select"
          )
          ?.focus();
      });
    }
  }

  function close(): void {
    if (typeof document === "undefined") {
      return;
    }

    if (isOpen()) {
      document.removeEventListener("keydown", handleEsc);
      document.body.classList.remove("noscroll");
      setIsOpen(false);
    }
  }

  function toggle(): void {
    if (isOpen()) close();
    else open();
  }

  onCleanup(() => {
    if (typeof document === "undefined") {
      return;
    }

    document.removeEventListener("keydown", handleEsc);
    document.body.classList.remove("noscroll");
  });

  const { SetSheetContext } = Context;

  SetSheetContext(() => {
    return {
      isOpen,
      isSheetTriggerButtonHovered,
      setSheetTriggerButtonHovered,
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

type TriggerProps = JSX.ButtonHTMLAttributes<HTMLButtonElement> & {
  children: JSXElement;
};

export function Trigger(_props: TriggerProps): JSXElement {
  const [props, rest] = splitProps(_props, ["children", "type"]);

  function toggle(): void {
    const { SheetContext } = Context;
    const c = SheetContext();
    c.toggle();
  }

  return (
    <button
      {...rest}
      type={props.type ?? "button"}
      onClick={toggle}
      onMouseEnter={() => {
        const { SheetContext } = Context;
        const c = SheetContext();
        c.setSheetTriggerButtonHovered(true);
      }}
      onFocus={() => {
        const { SheetContext } = Context;
        const c = SheetContext();
        c.setSheetTriggerButtonHovered(true);
      }}
      onMouseLeave={() => {
        const { SheetContext } = Context;
        const c = SheetContext();
        c.setSheetTriggerButtonHovered(false);
      }}
      onBlur={() => {
        const { SheetContext } = Context;
        const c = SheetContext();
        c.setSheetTriggerButtonHovered(false);
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
