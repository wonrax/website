import { LitElement, css, html } from "lit";
import { customElement, property } from "lit/decorators.js";

@customElement("code-group")
export class CodeGroup extends LitElement {
  // Define scoped styles right with your component, in plain CSS
  static styles = css`
    :host {
      all: initial;
    }
  `;

  // @property()
  // currentSlideIndex?: number = 0;

  slides: HTMLPreElement[] = [];

  titles: string[] = [];

  connectedCallback(): void {
    const slot = this.shadowRoot?.querySelector("slot");

    let rootNode: HTMLElement | null = null;

    slot?.assignedElements().forEach((node) => {
      if (
        typeof node.getAttribute("data-rehype-pretty-code-fragment") === null
      ) {
        return null;
      }

      // set the rootNode
      if (!rootNode) {
        rootNode = node.cloneNode() as HTMLElement;
      }

      if (node.children[0].classList.contains("code-block-title")) {
        let title = node.children[0].innerHTML.split("/");
        this.titles.push(title[title.length - 1]);

        // remove the title element
        node.removeChild(node.children[0]);

        this.slides.push(node.children[0] as HTMLPreElement);

        node.remove();
      }
    });

    if (rootNode) {
      (rootNode as HTMLElement).replaceChildren(...this.slides);
      console.log("rootNode", rootNode);
    }

    this.slides.slice(1).forEach((slide) => {
      slide.style.display = "none";
    });

    console.log("slides from constructor", this.slides);

    const shadow = this.attachShadow({ mode: "open" });

    const root = document.createElement("div");
    root.classList.add("feature-code");

    if (rootNode) {
      root.replaceChildren(rootNode);
    }

    shadow.appendChild(root as HTMLElement);
  }

  // Render the UI as a function of component state
  render() {
    return html`<slot />`;
  }
}
