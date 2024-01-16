// Disable solid eslint because this is a React component
// TODO figure out how to do this by eslint config
/* eslint-disable solid/no-destructure */
/* eslint-disable solid/no-react-specific-props */
/* eslint-disable solid/prefer-for */
/** @jsxImportSource react */

import React, { type ReactElement } from "react";
import parse from "html-react-parser";

// Inspired by
// https://github.com/delbaoliveira/website/blob/main/ui/Code.tsx
export default function CodeGroup({
  children,
}: {
  children: any;
}): ReactElement {
  // TODO read more on the docs to identify security issues
  // eslint-disable-next-line @typescript-eslint/no-unsafe-argument
  const content = parse(children.props.value.toString());

  const titles: string[] = [];
  const [slide, setSlide] = React.useState(0);

  const slides = React.Children.map(content, (child, index) => {
    if (
      !React.isValidElement(child) ||
      child.props?.["data-rehype-pretty-code-figure"] == null
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
            onClick={() => {
              setSlide(index);
            }}
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
