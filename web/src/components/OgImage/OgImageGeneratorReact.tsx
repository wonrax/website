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
): Promise<React.ReactElement> {
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
            lineHeight: 1.2,
            color: "#333",
            fontWeight: 500,
          }}
        >
          {props.title}
        </h1>
        {props.description != null ? (
          <p
            style={{
              margin: "1em 0 0 0",
              color: "#999",
              lineHeight: 1.3,
              fontSize: 36,
              letterSpacing: "-0.035em",
              fontWeight: 500,
            }}
          >
            {props.description}
          </p>
        ) : null}
        <div style={{ flexGrow: 1 }}></div>
        {import.meta.env.SITE ? (
          <p
            style={{
              margin: "0.5em 0 0 0",
              color: "#333",
              lineHeight: 1.2,
              fontSize: 48,
              letterSpacing: "-0.025em",
              fontWeight: 500,
            }}
          >
            {import.meta.env.SITE.replace(/^https?:\/\//, "")}
          </p>
        ) : null}
      </div>
    </div>
  );
}
