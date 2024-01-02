import {
  createSignal,
  createEffect,
  createResource,
  useContext,
  type Accessor,
  createRoot,
} from "solid-js";

type ContextType = {
  isOpen: Accessor<boolean>;
  toggle: () => void;
};

function createContext() {
  const [context, setContext] = createSignal<ContextType>({
    isOpen: () => false,
    toggle: () => {
      console.log("toggle default");
    },
  });
  return { SheetContext: context, SetSheetContext: setContext };
}

export const Context = createRoot(createContext);
