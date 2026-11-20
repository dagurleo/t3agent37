import icon from "./assets/icon.png";
import { Together } from "./Together";
import { Hero } from "./Hero";
import { Relay } from "./Relay";
import { Pricing } from "./Pricing";
import { InstallButton } from "./InstallButton";
import { AppleMark, GitHubMark, T3CodeMark } from "./logos";
import { AGENT37, REPO, T3BOAT, T3CODE } from "./links";

/** The one system t3agent37 runs on, as its mark and version. */
function MacOnly({ className }: { className: string }) {
  return (
    <span className={className}>
      <AppleMark aria-hidden />
      macOS 13+
    </span>
  );
}

function Wordmark({ withIcon = false }: { withIcon?: boolean }) {
  return (
    <a className="wordmark" href="/">
      {withIcon && <img src={icon} alt="" width={40} height={40} />}
      t3agent37
    </a>
  );
}

/** How to build it by hand, as the README's Install section has it. */
const BUILD = `git clone ${REPO}
cd t3agent37/apps/desktop
scripts/bundle-macos.sh --open`;

export function Site() {
  return (
    <>
      <Hero
        nav={
          <>
            <Wordmark />
            <div className="nav-links">
              <a href="#how-it-works">How it works</a>
              <a href="#pricing">Pricing</a>
              <a className="nav-btn" href={REPO}>
                <GitHubMark aria-hidden />
                GitHub
              </a>
            </div>
          </>
        }
      >
        {/* data-ai-content: what agents read in place of the marks (scripts/prerender.ts) */}
        <h1 data-ai-content="# Run your T3 Code agents in the cloud, on Agent37">
          Run your{" "}
          <span className="h1-t3" role="img" aria-label="T3 Code">
            <T3CodeMark aria-hidden />
            Code
          </span>{" "}
          agents in the cloud, on <span className="h1-agent37">Agent37</span>
        </h1>
        <p>
          t3agent37 creates machines on your Agent37 Cloud account and adds them to T3 Code. They sleep when idle and bill
          only their disk, so you pay for compute only while your agents run.
        </p>
        <div className="hero-cta">
          <InstallButton />
          <span className="hero-meta" data-ai-content="macOS 13 or later. Build from source.">
            <MacOnly className="hero-os" />
            <span>Build from source</span>
          </span>
        </div>
      </Hero>

      <Together />

      <Relay />

      <Pricing />

      <section className="needs" id="install">
        <p className="eyebrow">Before you install</p>
        <h2>What you need</h2>
        <ul className="needs-list">
          <li data-ai-content={`[T3 Code](${T3CODE}): the desktop app`}>
            <a href={T3CODE} className="needs-logo" aria-label="T3 Code">
              <T3CodeMark className="needs-t3" aria-hidden />
              Code
            </a>
            <span>The desktop app</span>
          </li>
          <li data-ai-content={`An [Agent37 Cloud](${AGENT37}) account and an API key. The key stays on your Mac, never on a machine.`}>
            <a href={AGENT37} className="needs-logo">
              Agent37
            </a>
            <span>A Cloud account and an API key. The key stays on your Mac, never on a machine.</span>
          </li>
          <li data-ai-content="A Mac on macOS 13 or later, with Rust and Xcode's command line tools to build it">
            <MacOnly className="needs-logo needs-os" />
            <span>A Mac, with Rust and Xcode's command line tools to build it</span>
          </li>
        </ul>

        <div className="build">
          <div className="build-copy">
            <h3>Build it from source</h3>
            <p>
              There's no prebuilt download yet. Hand the job to your coding agent, or run the build yourself: the app lands
              in your menu bar.
            </p>
            <InstallButton />
          </div>
          <pre className="build-code" aria-label="Build commands">
            <code>{BUILD}</code>
          </pre>
        </div>
      </section>

      <footer className="footer">
        <div className="footer-main" data-ai-ignore>
          <Wordmark withIcon />
          <nav className="footer-links" aria-label="Elsewhere">
            <a href={T3CODE} aria-label="T3 Code">
              <T3CodeMark className="footer-t3" aria-hidden />
              Code
            </a>
            <a href={AGENT37}>Agent37</a>
            <a href={REPO}>
              <GitHubMark aria-hidden />
              GitHub
            </a>
          </nav>
        </div>
        <div className="footer-fine">
          <p>
            A fork of <a href={T3BOAT}>t3boat by Pau Kraft</a>. MIT licensed.
          </p>
          <p>Not affiliated with or endorsed by T3 Code, Agent37 or Boat.</p>
        </div>
      </footer>
    </>
  );
}
