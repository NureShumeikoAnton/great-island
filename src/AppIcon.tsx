import { useId } from "react";

/** The app's icon, drawn from assets/icon.svg. */
export function AppIcon({ className }: { className?: string }) {
  // Two icons on one page must not share a gradient id.
  const gradient = useId();
  return (
    <svg className={className} viewBox="0 0 1024 1024" aria-hidden="true">
      <defs>
        <linearGradient id={gradient} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#FFB340" />
          <stop offset="1" stopColor="#FF7A00" />
        </linearGradient>
      </defs>
      <rect x="32" y="32" width="960" height="960" rx="220" fill={`url(#${gradient})`} />
      <rect x="172" y="392" width="680" height="240" rx="120" fill="#000" />
      <circle cx="312" cy="512" r="52" fill="none" stroke="#FF9F0A" strokeWidth="26" />
      <circle cx="452" cy="512" r="52" fill="none" stroke="#64D2FF" strokeWidth="26" />
      <rect x="560" y="490" width="180" height="44" rx="22" fill="#F5F5F7" />
    </svg>
  );
}
