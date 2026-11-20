import { DOWNLOAD } from "./links";

/** The newest release's DMG, from GitHub: signed, notarized, and it updates itself. */
export function DownloadButton() {
  return (
    <a className="install" href={DOWNLOAD}>
      <svg className="install-icon" viewBox="0 0 16 16" aria-hidden>
        <path d="M7 1h2v8H7zM3 6h2v2H3zM5 8h2v2H5zM7 10h2v2H7zM9 8h2v2H9zM11 6h2v2h-2zM1 11h2v3h10v-3h2v4H1z" />
      </svg>
      Download for Mac
    </a>
  );
}
