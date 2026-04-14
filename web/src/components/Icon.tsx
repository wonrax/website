import { splitProps, type Component, type JSX } from "solid-js";

type IconProps = JSX.SvgSVGAttributes<SVGSVGElement> & {
  size?: number | string;
  strokeWidth?: number | string;
};

function createIcon(
  paths: JSX.Element,
  viewBox = "0 0 24 24"
): Component<IconProps> {
  return function Icon(props: IconProps) {
    const [local, rest] = splitProps(props, ["size", "strokeWidth"]);

    return (
      <svg
        xmlns="http://www.w3.org/2000/svg"
        width={local.size ?? 24}
        height={local.size ?? 24}
        viewBox={viewBox}
        fill="none"
        stroke="currentColor"
        stroke-width={local.strokeWidth ?? 2}
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
        {...rest}
      >
        {paths}
      </svg>
    );
  };
}

export const ArrowUp = createIcon(
  <>
    <path d="m5 12 7-7 7 7" />
    <path d="M12 19V5" />
  </>
);

export const ArrowDown = createIcon(
  <>
    <path d="M12 5v14" />
    <path d="m19 12-7 7-7-7" />
  </>
);

export const X = createIcon(
  <>
    <path d="M18 6 6 18" />
    <path d="m6 6 12 12" />
  </>
);

export const MessagesSquare = createIcon(
  <>
    <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
    <path d="M8 10h8" />
    <path d="M8 7h6" />
  </>
);

export const User = createIcon(
  <>
    <path d="M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2" />
    <circle cx="12" cy="7" r="4" />
  </>
);
