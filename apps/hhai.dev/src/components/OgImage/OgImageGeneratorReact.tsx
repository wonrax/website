// Disable solid eslint because this is a React component
// TODO figure out how to do this by eslint config
/* eslint-disable solid/no-destructure */
/* eslint-disable solid/style-prop */
/** @jsxImportSource react */

import React, { type ReactElement } from "react";

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
        flexDirection: "column",
        height: "100%",
        width: "100%",
        padding: "36px 96px",
        justifyContent: "center",
        fontFamily: 'Inter, "Material Icons"',
        fontSize: 36,
        backgroundColor: "white",
      }}
    >
      <Logo style={{ color: "#111", marginLeft: -16 }} />
      <h1
        style={{
          letterSpacing: "-0.035em",
          margin: "0px",
          fontSize: 64,
          color: "#333",
        }}
      >
        {title}
      </h1>
      {description != null ? (
        <p style={{ margin: "16px 0 0 0", color: "#555" }}>{description}</p>
      ) : null}
      <p style={{ margin: "48px 0 0 0", color: "#111", fontWeight: 700 }}>
        hhai.dev
      </p>
    </div>
  );
}

const Logo = (props: any): ReactElement => (
  <svg
    width="128"
    height="128"
    viewBox="0 0 32 32"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    {...props}
  >
    {/* <rect width="32" height="32" rx="4.86957" fill="white" /> */}
    <path
      d="M6.72461 8.69566C7.28113 9.15943 7.83765 9.6232 9.04345 9.6232C11.3623 9.6232 11.3623 7.76813 13.6811 7.76813C16.0927 7.76813 15.9072 9.6232 18.3188 9.6232C20.6377 9.6232 20.6377 7.76813 22.9565 7.76813C24.1623 7.76813 24.7188 8.2319 25.2753 8.69566"
      stroke="currentColor"
      stroke-width="2.43478"
      stroke-linecap="round"
      stroke-linejoin="round"
    />
    <path
      d="M6.72461 16C7.28113 16.4638 7.83765 16.9275 9.04345 16.9275C11.3623 16.9275 11.3623 15.0724 13.6811 15.0724C16.0927 15.0724 15.9072 16.9275 18.3188 16.9275C20.6377 16.9275 20.6377 15.0724 22.9565 15.0724C24.1623 15.0724 24.7188 15.5362 25.2753 16"
      stroke="currentColor"
      stroke-width="2.43478"
      stroke-linecap="round"
      stroke-linejoin="round"
    />
    <path
      d="M6.72461 23.3043C7.28113 23.7681 7.83765 24.2318 9.04345 24.2318C11.3623 24.2318 11.3623 22.3768 13.6811 22.3768C16.0927 22.3768 15.9072 24.2318 18.3188 24.2318C20.6377 24.2318 20.6377 22.3768 22.9565 22.3768C24.1623 22.3768 24.7188 22.8405 25.2753 23.3043"
      stroke="currentColor"
      stroke-width="2.43478"
      stroke-linecap="round"
      stroke-linejoin="round"
    />
  </svg>
);
