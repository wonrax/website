// Disable solid eslint because this is a React component
// TODO figure out how to do this by eslint config
/* eslint-disable solid/no-destructure */
/* eslint-disable solid/style-prop */
/** @jsxImportSource react */

import React from "react";

interface Props {
  title: string;
  description?: string;
  image?: string;
  url?: string;
}

export default async function OgImage({
  title,
  description,
}: Props): Promise<
  React.ReactElement<any, string | React.JSXElementConstructor<any>>
> {
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
          flexGrow: 1,
          height: "100%",
          width: "450px",
          maxWidth: "450px",
          background:
            "radial-gradient(circle farthest-corner at 50% 100%, rgba(255,255,255,1) 5%, rgba(0,0,0,0.4) 5%, rgba(0,0,0,0.4) 10%, rgba(255,255,255,1) 10%, rgba(255,255,255,1) 15%, rgba(0,0,0,0.4) 15%, rgba(0,0,0,0.4) 20%, rgba(255,255,255,1) 20%, rgba(255,255,255,1) 25%, rgba(0,0,0,0.4) 25%, rgba(0,0,0,0.4) 30%, rgba(255,255,255,1) 30%, rgba(255,255,255,1) 35%, rgba(0,0,0,0.4) 35%, rgba(0,0,0,0.4) 40%, rgba(0,0,0,0) 40%), radial-gradient(circle at 100% 50%, rgba(255,255,255,1) 5%, rgba(0,0,0,0.4) 5%, rgba(0,0,0,0.4) 10%, rgba(255,255,255,1) 10%, rgba(255,255,255,1) 15%, rgba(0,0,0,0.4) 15%, rgba(0,0,0,0.4) 20%, rgba(255,255,255,1) 20%, rgba(255,255,255,1) 25%, rgba(0,0,0,0.4) 25%, rgba(0,0,0,0.4) 30%, rgba(255,255,255,1) 30%, rgba(255,255,255,1) 35%, rgba(0,0,0,0.4) 35%, rgba(0,0,0,0.4) 40%, rgba(0,0,0,0) 40%), radial-gradient(circle at 50% 0%, rgba(255,255,255,1) 5%, rgba(0,0,0,0.4) 5%, rgba(0,0,0,0.4) 10%, rgba(255,255,255,1) 10%, rgba(255,255,255,1) 15%, rgba(0,0,0,0.4) 15%, rgba(0,0,0,0.4) 20%, rgba(255,255,255,1) 20%, rgba(255,255,255,1) 25%, rgba(0,0,0,0.4) 25%, rgba(0,0,0,0.4) 30%, rgba(255,255,255,1) 30%, rgba(255,255,255,1) 35%, rgba(0,0,0,0.4) 35%, rgba(0,0,0,0.4) 40%, rgba(0,0,0,0) 40%), radial-gradient(circle at 0px 50%, rgba(255,255,255,1) 5%, rgba(0,0,0,0.4) 5%, rgba(0,0,0,0.4) 10%, rgba(255,255,255,1) 10%, rgba(255,255,255,1) 15%, rgba(0,0,0,0.4) 15%, rgba(0,0,0,0.4) 20%, rgba(255,255,255,1) 20%, rgba(255,255,255,1) 25%, rgba(0,0,0,0.4) 25%, rgba(0,0,0,0.4) 30%, rgba(255,255,255,1) 30%, rgba(255,255,255,1) 35%, rgba(0,0,0,0.4) 35%, rgba(0,0,0,0.4) 40%, rgba(0,0,0,0) 40%)",
          backgroundSize: "180px 180px, 180px 180px, 180px 180px, 180px 180px",
          backgroundPosition: "0% 0%, 0% 0%, 0% 0%, 0% 0%",
          backgroundRepeat: "repeat, repeat, repeat, repeat",
          backgroundColor: "rgba(255,255,255,1)",
        }}
      />
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          padding: "0 96px 0 48px",
          flexShrink: 1,
          boxSizing: "border-box",
          justifyContent: "center",
        }}
      >
        <h1
          style={{
            letterSpacing: "-0.035em",
            margin: "0px",
            fontSize: description != null ? 48 : 82,
            lineHeight: 1.3,
            color: "#333",
            fontWeight: 500,
          }}
        >
          {title}
        </h1>
        {description != null ? (
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
            {description}
          </p>
        ) : null}
      </div>
    </div>
  );
}
