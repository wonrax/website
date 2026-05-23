import {
  For,
  Show,
  createMemo,
  createSignal,
  createUniqueId,
  onCleanup,
  onMount,
  type JSXElement,
} from "solid-js";
import { CODE_GROUP_TITLE_ATTRIBUTE } from "@/../plugins/shared";

type CodeGroupPane = {
  contentNode: Element;
  title: string;
  language: string;
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

  // rehype-pretty-code puts data-language on the <pre>, not the wrapping
  // <figure> — read it from the pre so the tab strip can show the lang label.
  const pre = child.querySelector("pre");
  const language = pre?.getAttribute("data-language") ?? "";

  return {
    title: child.getAttribute(CODE_GROUP_TITLE_ATTRIBUTE) ?? "",
    language,
    contentNode: child,
  };
}

function getPaneText(pane: CodeGroupPane | undefined): string {
  if (pane == null) return "";
  const code = pane.contentNode.querySelector("pre > code");
  return code?.textContent ?? "";
}

export default function CodeGroup(props: { children: JSXElement }): JSXElement {
  const [hydrated, setHydrated] = createSignal(false);
  const [currentSlide, setCurrentSlide] = createSignal(0);
  const [panes, setPanes] = createSignal<CodeGroupPane[]>([]);
  const [copied, setCopied] = createSignal(false);
  const groupId = createUniqueId();
  let copyResetTimer: ReturnType<typeof setTimeout> | undefined;

  onMount(() => {
    if (!(props.children instanceof HTMLElement)) {
      return;
    }

    const nextPanes = Array.from(props.children.children).map(extractPane);

    setPanes(nextPanes);
    setHydrated(true);
  });

  onCleanup(() => {
    if (copyResetTimer != null) clearTimeout(copyResetTimer);
  });

  const activePane = createMemo(() => panes()[currentSlide()]);

  const handleCopy = (): void => {
    const text = getPaneText(activePane());
    if (text === "") return;
    void navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      if (copyResetTimer != null) clearTimeout(copyResetTimer);
      copyResetTimer = setTimeout(() => setCopied(false), 1500);
    });
  };

  return (
    <>
      <Show when={!hydrated()}>{props.children}</Show>
      <Show when={hydrated()}>
        <figure data-rehype-pretty-code-figure class="code-group">
          <div
            class="ui-meta code-group-tabs"
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
            <span class="code-group-tabs__spacer" />
            <Show when={activePane()?.language}>
              <span class="code-group-tabs__lang">
                {activePane()?.language}
              </span>
            </Show>
            <button
              type="button"
              class="ui-button ui-button--xs code-group-tabs__copy"
              data-copied={copied() ? "true" : "false"}
              onClick={handleCopy}
              aria-label="Copy code"
            >
              {copied() ? "copied" : "copy"}
            </button>
          </div>
          <For each={panes()}>
            {(pane, index) => (
              <div
                role="tabpanel"
                id={`${groupId}-panel-${index()}`}
                aria-labelledby={`${groupId}-tab-${index()}`}
                hidden={currentSlide() !== index()}
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
