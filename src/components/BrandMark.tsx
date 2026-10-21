type BrandMarkProps = {
  size?: number;
  className?: string;
  title?: string;
};

/**
 * EasyLauncher brand mark: two merged peaks with a low sun.
 * Drawn as a single filled silhouette so it stays legible down to 16px.
 */
export function BrandMark({ size = 24, className, title }: BrandMarkProps) {
  return (
    <svg
      aria-hidden={title ? undefined : true}
      aria-label={title}
      className={className}
      fill="none"
      height={size}
      role={title ? "img" : undefined}
      viewBox="0 0 32 32"
      width={size}
    >
      <circle cx="24.2" cy="8" fill="currentColor" opacity="0.55" r="2.5" />
      <path
        d="M4 25.6 11.5 11.5c.6-1.1 1.8-1.1 2.4 0l3.2 6.1c.5-.6 1.2-.6 1.7 0l1.9-3.3c.5-.9 1.6-.9 2.1 0l5.2 11.3c.2.4-.1.9-.5.9H4.5c-.5 0-.8-.5-.5-.9Z"
        fill="currentColor"
      />
    </svg>
  );
}
