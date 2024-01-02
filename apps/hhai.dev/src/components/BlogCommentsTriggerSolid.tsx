import { createEffect, useContext, type JSXElement } from "solid-js";
import { Context } from "./BlogCommentsContextSolid";

export default function Trigger({
  children,
  ...rest
}: {
  children: JSXElement;
  class?: string;
}) {
  function toggle() {
    const { SheetContext, SetSheetContext } = Context;
    const c = SheetContext();
    c.toggle();
  }

  return (
    <button {...rest} onClick={toggle}>
      {children}
    </button>
  );
}
