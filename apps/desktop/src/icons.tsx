import type { ReactNode, SVGProps } from "react";

export type IconProps = Omit<SVGProps<SVGSVGElement>, "children"> & { size?: number };

function Glyph({ size = 18, children, ...rest }: IconProps & { children: ReactNode }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.6}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      {...rest}
    >
      {children}
    </svg>
  );
}

export function IconBook(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M4 5.6c2.4-.9 4.9-.8 7.2.6v13c-2.3-1.4-4.8-1.5-7.2-.6z" />
      <path d="M20 5.6c-2.4-.9-4.9-.8-7.2.6v13c2.3-1.4 4.8-1.5 7.2-.6z" />
      <path d="M12 6.2v13" />
    </Glyph>
  );
}

export function IconSigma(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M17.5 5H7l5.8 7L7 19h10.5" />
    </Glyph>
  );
}

export function IconAtom(props: IconProps) {
  return (
    <Glyph {...props}>
      <circle cx="12" cy="12" r="1.3" fill="currentColor" stroke="none" />
      <ellipse cx="12" cy="12" rx="9" ry="3.6" />
      <ellipse cx="12" cy="12" rx="9" ry="3.6" transform="rotate(60 12 12)" />
      <ellipse cx="12" cy="12" rx="9" ry="3.6" transform="rotate(120 12 12)" />
    </Glyph>
  );
}

export function IconFlask(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M9.5 3h5" />
      <path d="M10.5 3v5.2L5.4 17a2.2 2.2 0 0 0 1.9 3.3h9.4a2.2 2.2 0 0 0 1.9-3.3L13.5 8.2V3" />
      <path d="M7.6 14.5h8.8" />
    </Glyph>
  );
}

export function IconQuote(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M9.5 6.5H5.8A1.8 1.8 0 0 0 4 8.3v3.4a1.8 1.8 0 0 0 1.8 1.8h2.4c0 2.2-1.4 3.4-3.2 4" />
      <path d="M19.5 6.5h-3.7a1.8 1.8 0 0 0-1.8 1.8v3.4a1.8 1.8 0 0 0 1.8 1.8h2.4c0 2.2-1.4 3.4-3.2 4" />
    </Glyph>
  );
}

export function IconPlus(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M12 5v14M5 12h14" />
    </Glyph>
  );
}

export function IconArrowUp(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M12 19V5" />
      <path d="m6 11 6-6 6 6" />
    </Glyph>
  );
}

export function IconArrowRight(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M5 12h14" />
      <path d="m13 6 6 6-6 6" />
    </Glyph>
  );
}

export function IconSliders(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M4 7h9M17 7h3" />
      <circle cx="15" cy="7" r="2" />
      <path d="M4 17h3M11 17h9" />
      <circle cx="9" cy="17" r="2" />
    </Glyph>
  );
}

export function IconPuzzle(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M10 4.5a2 2 0 0 1 4 0V6h2.5A1.5 1.5 0 0 1 18 7.5V10h1.5a2 2 0 0 1 0 4H18v2.5a1.5 1.5 0 0 1-1.5 1.5H14v-1.5a2 2 0 0 0-4 0V18H7.5A1.5 1.5 0 0 1 6 16.5V14H4.5a2 2 0 0 1 0-4H6V7.5A1.5 1.5 0 0 1 7.5 6H10z" />
    </Glyph>
  );
}

export function IconDatabase(props: IconProps) {
  return (
    <Glyph {...props}>
      <ellipse cx="12" cy="5.5" rx="7.5" ry="2.8" />
      <path d="M4.5 5.5v13c0 1.5 3.4 2.8 7.5 2.8s7.5-1.3 7.5-2.8v-13" />
      <path d="M4.5 12c0 1.5 3.4 2.8 7.5 2.8s7.5-1.3 7.5-2.8" />
    </Glyph>
  );
}

export function IconFolder(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M3.5 6.5A1.5 1.5 0 0 1 5 5h4l2 2.5h8a1.5 1.5 0 0 1 1.5 1.5v8A1.5 1.5 0 0 1 19 18.5H5A1.5 1.5 0 0 1 3.5 17z" />
    </Glyph>
  );
}

export function IconCheck(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="m5 12.5 4.5 4.5L19 7" />
    </Glyph>
  );
}

export function IconX(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="m6 6 12 12M18 6 6 18" />
    </Glyph>
  );
}

export function IconTrash(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M4 7h16" />
      <path d="M9 7V4.5h6V7" />
      <path d="M6.5 7l.8 12.2a1.5 1.5 0 0 0 1.5 1.3h6.4a1.5 1.5 0 0 0 1.5-1.3L17.5 7" />
      <path d="M10 11v6M14 11v6" />
    </Glyph>
  );
}

export function IconRefresh(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M20 12a8 8 0 1 1-2.3-5.6" />
      <path d="M20 4v5h-5" />
    </Glyph>
  );
}

export function IconChevronDown(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="m6 9 6 6 6-6" />
    </Glyph>
  );
}

export function IconChevronRight(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="m9 6 6 6-6 6" />
    </Glyph>
  );
}

export function IconClock(props: IconProps) {
  return (
    <Glyph {...props}>
      <circle cx="12" cy="12" r="8.5" />
      <path d="M12 7.5V12l3 2" />
    </Glyph>
  );
}

export function IconKey(props: IconProps) {
  return (
    <Glyph {...props}>
      <circle cx="8" cy="8" r="3.5" />
      <path d="m10.5 10.5 8 8" />
      <path d="m15.5 15.5 2-2" />
      <path d="m18 18 2-2" />
    </Glyph>
  );
}

export function IconTerminal(props: IconProps) {
  return (
    <Glyph {...props}>
      <rect x="3" y="4.5" width="18" height="15" rx="2" />
      <path d="m7 9 3 3-3 3" />
      <path d="M13 15h4" />
    </Glyph>
  );
}

export function IconPlug(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M9 3v5M15 3v5" />
      <path d="M6.5 8h11v3.5a5.5 5.5 0 0 1-11 0z" />
      <path d="M12 17v4" />
    </Glyph>
  );
}

export function IconInfo(props: IconProps) {
  return (
    <Glyph {...props}>
      <circle cx="12" cy="12" r="8.5" />
      <path d="M12 11v5" />
      <path d="M12 8h.01" />
    </Glyph>
  );
}

export function IconLock(props: IconProps) {
  return (
    <Glyph {...props}>
      <rect x="5" y="10" width="14" height="10" rx="2" />
      <path d="M8.5 10V7.5a3.5 3.5 0 0 1 7 0V10" />
    </Glyph>
  );
}

export function IconPen(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M4 20h4L18.5 9.5a2.1 2.1 0 0 0-3-3L5 17v3z" />
      <path d="m14.5 6.5 3 3" />
    </Glyph>
  );
}

export function IconEye(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M2.8 12S6.4 5.8 12 5.8 21.2 12 21.2 12 17.6 18.2 12 18.2 2.8 12 2.8 12z" />
      <circle cx="12" cy="12" r="2.8" />
    </Glyph>
  );
}

export function IconEyeOff(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M4 4l16 16" />
      <path d="M9.9 5.9A9.7 9.7 0 0 1 12 5.8c5.6 0 9.2 6.2 9.2 6.2a17 17 0 0 1-2.4 3.2" />
      <path d="M6.1 7.8A17.4 17.4 0 0 0 2.8 12S6.4 18.2 12 18.2c1 0 2-.2 2.8-.5" />
      <path d="M9.9 9.9a3 3 0 0 0 4.2 4.2" />
    </Glyph>
  );
}

export function subjectIcon(id: string, size = 18) {
  switch (id) {
    case "math":
      return <IconSigma size={size} />;
    case "physics":
      return <IconAtom size={size} />;
    case "chemistry":
      return <IconFlask size={size} />;
    case "english":
      return <IconQuote size={size} />;
    default:
      return <IconBook size={size} />;
  }
}
