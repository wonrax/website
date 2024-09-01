// Disable solid eslint because this is a React component
// TODO figure out how to do this by eslint config

/** @jsxImportSource react */

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
        "flex-direction": "row",
        height: "100%",
        width: "100%",
        "font-family": 'Inter, "Material Icons"',
        "font-size": 36,
        "background-color": "white",
        "box-sizing": "border-box",
      }}
    >
      <div
        style={{
          "flex-grow": 1,
          height: "100%",
          width: "450px",
          "max-width": "450px",
          background:
            "radial-gradient(circle farthest-corner at 50% 100%, rgba(255,255,255,1) 5%, rgba(0,0,0,0.4) 5%, rgba(0,0,0,0.4) 10%, rgba(255,255,255,1) 10%, rgba(255,255,255,1) 15%, rgba(0,0,0,0.4) 15%, rgba(0,0,0,0.4) 20%, rgba(255,255,255,1) 20%, rgba(255,255,255,1) 25%, rgba(0,0,0,0.4) 25%, rgba(0,0,0,0.4) 30%, rgba(255,255,255,1) 30%, rgba(255,255,255,1) 35%, rgba(0,0,0,0.4) 35%, rgba(0,0,0,0.4) 40%, rgba(0,0,0,0) 40%), radial-gradient(circle at 100% 50%, rgba(255,255,255,1) 5%, rgba(0,0,0,0.4) 5%, rgba(0,0,0,0.4) 10%, rgba(255,255,255,1) 10%, rgba(255,255,255,1) 15%, rgba(0,0,0,0.4) 15%, rgba(0,0,0,0.4) 20%, rgba(255,255,255,1) 20%, rgba(255,255,255,1) 25%, rgba(0,0,0,0.4) 25%, rgba(0,0,0,0.4) 30%, rgba(255,255,255,1) 30%, rgba(255,255,255,1) 35%, rgba(0,0,0,0.4) 35%, rgba(0,0,0,0.4) 40%, rgba(0,0,0,0) 40%), radial-gradient(circle at 50% 0%, rgba(255,255,255,1) 5%, rgba(0,0,0,0.4) 5%, rgba(0,0,0,0.4) 10%, rgba(255,255,255,1) 10%, rgba(255,255,255,1) 15%, rgba(0,0,0,0.4) 15%, rgba(0,0,0,0.4) 20%, rgba(255,255,255,1) 20%, rgba(255,255,255,1) 25%, rgba(0,0,0,0.4) 25%, rgba(0,0,0,0.4) 30%, rgba(255,255,255,1) 30%, rgba(255,255,255,1) 35%, rgba(0,0,0,0.4) 35%, rgba(0,0,0,0.4) 40%, rgba(0,0,0,0) 40%), radial-gradient(circle at 0px 50%, rgba(255,255,255,1) 5%, rgba(0,0,0,0.4) 5%, rgba(0,0,0,0.4) 10%, rgba(255,255,255,1) 10%, rgba(255,255,255,1) 15%, rgba(0,0,0,0.4) 15%, rgba(0,0,0,0.4) 20%, rgba(255,255,255,1) 20%, rgba(255,255,255,1) 25%, rgba(0,0,0,0.4) 25%, rgba(0,0,0,0.4) 30%, rgba(255,255,255,1) 30%, rgba(255,255,255,1) 35%, rgba(0,0,0,0.4) 35%, rgba(0,0,0,0.4) 40%, rgba(0,0,0,0) 40%)",
          "background-size":
            "180px 180px, 180px 180px, 180px 180px, 180px 180px",
          "background-position": "0% 0%, 0% 0%, 0% 0%, 0% 0%",
          "background-repeat": "repeat, repeat, repeat, repeat",
          "background-color": "rgba(255,255,255,1)",
        }}
      />
      <div
        style={{
          display: "flex",
          "flex-direction": "column",
          padding: "0 96px 0 48px",
          "flex-shrink": 1,
          "box-sizing": "border-box",
          "justify-content": "center",
        }}
      >
        <h1
          style={{
            "letter-spacing": "-0.035em",
            margin: "0px",
            "font-size": props.description != null ? 48 : 82,
            "line-height": 1.3,
            color: "#333",
            "font-weight": 500,
          }}
        >
          {props.title}
        </h1>
        {props.description != null ? (
          <p
            style={{
              margin: "0",
              color: "#777",
              "line-height": 1.3,
              "font-size": 48,
              "letter-spacing": "-0.035em",
              "font-weight": 500,
            }}
          >
            {props.description}
          </p>
        ) : null}
      </div>
    </div>
  );
}
