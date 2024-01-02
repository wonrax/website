import { createEffect, useContext } from "solid-js";
import { Context } from "./BlogCommentsContextSolid";

export default function Trigger() {
  function toggle() {
    const { SheetContext, SetSheetContext } = Context;
    const c = SheetContext();
    console.log("context from trigger", c);
    c.toggle();
  }

  return <button onClick={toggle}>Open</button>;
}
