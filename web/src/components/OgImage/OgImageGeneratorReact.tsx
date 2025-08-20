/** @jsxImportSource react */

// TODO: the image is broken

import React from "react";

interface Props {
  title: string;
  description?: string;
  image?: string;
  url?: string;
}

export default async function OgImage(
  props: Props
): Promise<React.ReactElement<any, string | React.JSXElementConstructor<any>>> {
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "row",
        height: "100%",
        width: "100%",
        fontFamily: 'Inter, "Material Icons"',
        fontSize: 36,
        backgroundColor: "white",
        boxSizing: "border-box",
      }}
    >
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          // padding: "48px 96px 48px 48px",
          padding: "96px",
          flexShrink: 1,
          boxSizing: "border-box",
          justifyContent: "center",
        }}
      >
        <h1
          style={{
            letterSpacing: "-0.035em",
            margin: "0px",
            fontSize: props.description != null ? 64 : 96,
            lineHeight: 1.4,
            color: "#333",
            fontWeight: 500,
          }}
        >
          {props.title}
        </h1>
        {props.description != null ? (
          <p
            style={{
              margin: "0",
              color: "#777",
              lineHeight: 1.3,
              fontSize: 48,
              letterSpacing: "-0.035em",
              fontWeight: 500,
            }}
          >
            {props.description}
          </p>
        ) : null}
      </div>
    </div>
  );
}
