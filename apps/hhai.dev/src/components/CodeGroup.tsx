/** @jsxImportSource react */

import React from "react";
import parse from "html-react-parser";

// Inspired by
// https://github.com/delbaoliveira/website/blob/main/ui/Code.tsx
export default function CodeGroup({ children }) {
  const content = parse(children.props.value.toString());

  let titles: string[] = [];
  const [slide, setSlide] = React.useState(0);

  const slides = React.Children.map(content, (child, index) => {
    if (
      !React.isValidElement(child) ||
      typeof child.props?.["data-rehype-pretty-code-figure"] === "undefined"
    ) {
      return null;
    }

    if (
      child.props.children?.[0]?.props?.["className"] === "code-block-title"
    ) {
      let title = child.props.children[0].props.children.split("/");
      titles.push(title[title.length - 1]);

      // remove the title element
      child.props.children.shift();
    }

    return (
      <div
        key={index}
        style={index === slide ? { display: "block" } : { display: "none" }}
      >
        {child.props.children}
      </div>
    );
  });
  return (
    <figure data-rehype-pretty-code-figure className="code-group">
      <div className="code-group-tabs">
        {titles.map((title, index) => (
          <div
            onClick={() => setSlide(index)}
            className={"code-block-title" + (index === slide ? " active" : "")}
            key={index}
          >
            {title}
          </div>
        ))}
      </div>
      {slides}
    </figure>
  );
}
