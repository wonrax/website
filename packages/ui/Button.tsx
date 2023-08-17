import * as React from "react";

export const Button = ({ children }: { children: React.ReactNode }) => {
  return (
    <button className="bg-red-500" onClick={() => console.log("he")}>
      {children}
    </button>
  );
};
