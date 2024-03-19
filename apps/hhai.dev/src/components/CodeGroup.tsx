import {
  For,
  createEffect,
  type JSXElement,
  createSignal,
  Show,
} from "solid-js";

export default function CodeGroup(props: { children: any }): JSXElement {
  const [hydrated, setHydrated] = createSignal(false);
  const titles: string[] = [];
  const [currentSlide, setCurrentSlide] = createSignal<number>(0);

  createEffect(() => {
    if (props.children instanceof HTMLElement) {
      const newChildren: Element[] = [];
      for (const child of props.children.children) {
        if (child.tagName === "figure") {
          if (
            child.attributes.getNamedItem("data-rehype-pretty-code-figure") ==
            null
          )
            throw new Error(
              `CodeGroupSolid: figure element must have \
              data-rehype-pretty-code-figure attribute`,
            );
        }

        if (
          child.children.length > 0 ||
          child.children[0].classList.contains("code-block-title")
        ) {
          const title = child.children[0].textContent?.split("/");
          if (title != null) titles.push(title[title.length - 1]);

          // remove the title element
          child.removeChild(child.children[0]);
        }

        newChildren.push(child);
      }

      setHydrated(true);
    }
  });

  return (
    <>
      <Show when={!hydrated()}>{props.children}</Show>
      <Show when={hydrated()}>
        <figure data-rehype-pretty-code-figure class="code-group">
          <div class="code-group-tabs">
            <For each={titles}>
              {(title, index) => (
                <div
                  onClick={() => {
                    setCurrentSlide(index);
                  }}
                  class={
                    "code-block-title" +
                    (currentSlide() === index() ? " active" : "")
                  }
                >
                  {title}
                </div>
              )}
            </For>
          </div>
          {/* {...props.children.children} */}
          <For each={props.children.children}>
            {(child, index) => (
              <div
                style={
                  currentSlide() === index()
                    ? { display: "block" }
                    : { display: "none" }
                }
              >
                <For each={child.children}>{(child) => child}</For>
              </div>
            )}
          </For>
        </figure>
      </Show>
    </>
  );
}
