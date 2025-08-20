import { createSignal, type Accessor, createRoot, type Setter } from "solid-js";

interface ContextType {
  isOpen: Accessor<boolean>;
  isSheetTriggerButtonHovered: Accessor<boolean>;
  setSheetTriggerButtonHovered: (s: boolean) => void;
  toggle: () => void;
  initialized: boolean;
}

// We're using signals to create context because Astro doesn't support context
// yet. Related: https://docs.astro.build/en/core-concepts/sharing-state and
// https://github.com/withastro/roadmap/discussions/742
function createCommentSheetContext(): {
  SheetContext: Accessor<ContextType>;
  SetSheetContext: Setter<ContextType>;
} {
  const [context, setContext] = createSignal<ContextType>({
    isOpen: () => {
      throw new Error("isOpen called before context was set");
    },
    isSheetTriggerButtonHovered: () => {
      throw new Error(
        "isSheetTriggerButtonHoveredcalled before context was set"
      );
    },
    setSheetTriggerButtonHovered: () => {
      throw new Error(
        "setSheetTriggerButtonHovered called before context was set"
      );
    },
    toggle: () => {
      throw new Error("toggle called before context was set");
    },
    initialized: false,
  });
  return { SheetContext: context, SetSheetContext: setContext };
}

const Context = createRoot(createCommentSheetContext);

export default Context;
