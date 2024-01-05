import { createSignal, type Accessor, createRoot } from "solid-js";

type ContextType = {
  isOpen: Accessor<boolean>;
  toggle: () => void;
  initialized: boolean;
};

// We're using signals to create a context because Astro doesn't support context
// yet. Related: https://docs.astro.build/en/core-concepts/sharing-state and
// https://github.com/withastro/roadmap/discussions/742
function createCommentSheetContext() {
  const [context, setContext] = createSignal<ContextType>({
    isOpen: () => {
      throw new Error("isOpen called before context was set");
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
