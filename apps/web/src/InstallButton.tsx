import { useEffect, useState } from "react";
import { REPO } from "./links";

const PROMPT = `Please build and install t3agent37 (${REPO}) on my Mac from source: clone it, follow "Build, install, test" in its CONTRIBUTING.md (it needs Rust and Xcode's command line tools), and run scripts/bundle-macos.sh --install in apps/desktop.`;

const LABELS = { idle: "Let your agent build it", copied: "Prompt copied. Paste it", failed: "Couldn't copy" };

/** Copies a prompt that has the user's coding agent build and install t3agent37 from source. */
export function InstallButton() {
  const [state, setState] = useState<keyof typeof LABELS>("idle");

  useEffect(() => {
    if (state === "idle") return;
    const t = setTimeout(() => setState("idle"), 2400);
    return () => clearTimeout(t);
  }, [state]);

  const copy = () =>
    navigator.clipboard.writeText(PROMPT).then(
      () => setState("copied"),
      () => setState("failed"),
    );

  // both labels stay laid out, so the button keeps its width while they cross-fade
  return (
    <button className="install" data-state={state} onClick={copy} type="button" aria-label={LABELS[state]}>
      <svg className="install-icon" viewBox="0 0 16 16" aria-hidden>
        <path className="install-copy" d="M5 1h10v10h-4v4H1V5h4V1Zm2 2v2h4v4h2V3H7ZM3 7v6h6V7H3Z" />
        <path className="install-check" d="M1 7h3v3H1zM3 9h3v3H3zM5 11h3v3H5zM7 9h3v3H7zM9 7h3v3H9zM11 5h3v3h-3zM13 3h3v3h-3z" />
      </svg>
      <span className="install-labels">
        <span className="install-label" aria-hidden>
          {LABELS.idle}
        </span>
        <span className="install-label" aria-hidden>
          {state === "failed" ? LABELS.failed : LABELS.copied}
        </span>
      </span>
    </button>
  );
}
