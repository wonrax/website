import {
  For,
  Show,
  createSignal,
  createUniqueId,
  onMount,
  type JSXElement,
} from "solid-js";
import { CODE_GROUP_TITLE_ATTRIBUTE } from "@/../plugins/shared";

type CodeGroupPane = {
  contentNode: Element;
  title: string;
};

function assertPrettyCodeFigure(element: Element): void {
  if (
    element.tagName === "FIGURE" &&
    element.getAttribute("data-rehype-pretty-code-figure") == null
  ) {
    throw new Error(
      "CodeGroupSolid: figure element must have data-rehype-pretty-code-figure attribute"
    );
  }
}

function extractPane(child: Element): CodeGroupPane {
  assertPrettyCodeFigure(child);

  return {
    title: child.getAttribute(CODE_GROUP_TITLE_ATTRIBUTE) ?? "",
    contentNode: child,
  };
}

export default function CodeGroup(props: { children: JSXElement }): JSXElement {
  const [hydrated, setHydrated] = createSignal(false);
  const [currentSlide, setCurrentSlide] = createSignal(0);
  const [panes, setPanes] = createSignal<CodeGroupPane[]>([]);
  const groupId = createUniqueId();

  onMount(() => {
    if (!(props.children instanceof HTMLElement)) {
      return;
    }

    const nextPanes = Array.from(props.children.children).map(extractPane);

    setPanes(nextPanes);
    setHydrated(true);
  });

  return (
    <>
      <Show when={!hydrated()}>{props.children}</Show>
      <Show when={hydrated()}>
        <figure data-rehype-pretty-code-figure class="code-group">
          <div
            class="code-group-tabs"
            role="tablist"
            aria-label="Code examples"
          >
            <For each={panes()}>
              {(pane, index) => (
                <button
                  type="button"
                  role="tab"
                  id={`${groupId}-tab-${index()}`}
                  aria-selected={currentSlide() === index()}
                  aria-controls={`${groupId}-panel-${index()}`}
                  tabIndex={currentSlide() === index() ? 0 : -1}
                  onClick={() => {
                    setCurrentSlide(index());
                  }}
                  class={
                    "code-block-title" +
                    (currentSlide() === index() ? " active" : "")
                  }
                >
                  {pane.title}
                </button>
              )}
            </For>
          </div>
          <For each={panes()}>
            {(pane, index) => (
              <div
                role="tabpanel"
                id={`${groupId}-panel-${index()}`}
                aria-labelledby={`${groupId}-tab-${index()}`}
                hidden={currentSlide() !== index()}
                style={
                  currentSlide() === index()
                    ? { display: "block" }
                    : { display: "none" }
                }
              >
                {pane.contentNode}
              </div>
            )}
          </For>
        </figure>
      </Show>
    </>
  );
}
