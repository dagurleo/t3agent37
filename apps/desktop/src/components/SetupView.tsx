import { useState } from "react";
import type { Setup, SetupAction } from "../types";
import { Icon } from "./Icons";
import { Dots } from "./common";

const STEPS: { id: Exclude<Setup, "starting" | "ready">; title: string; body: string }[] = [
  {
    id: "connectAgent37",
    title: "Connect Agent37 Cloud",
    body: "Paste an API key from your Agent37 dashboard. Machines bill to your own Agent37 wallet; the key stays on this computer.",
  },
  {
    id: "installT3",
    title: "Install T3 Code",
    body: "Your agents' chats live there. t3agent37 adds machines to it.",
  },
];

export function SetupView({
  setup,
  busy,
  onAction,
  onSaveKey,
}: {
  setup: Setup;
  busy: boolean;
  signInCode?: string | null;
  onAction: (a: SetupAction) => void;
  onSaveKey: (key: string) => Promise<void>;
}) {
  const [key, setKey] = useState("");
  if (setup === "starting" || setup === "ready") {
    return (
      <div className="view center">
        <p className="caption">
          <Dots /> checking your setup
        </p>
      </div>
    );
  }
  const current = STEPS.findIndex((s) => s.id === setup);
  const looksLikeKey = key.trim().startsWith("sk_live_");

  return (
    <div className="view">
      <div className="view-body">
        <h1 className="display">Get set up in two steps</h1>
        <p className="lede">Then launch machines for T3 Code that sleep when no agent is working, billing disk alone.</p>
        <ol className="steps">
          {STEPS.map((s, i) => {
            const status = i < current ? "done" : i === current ? "current" : "todo";
            return (
              <li key={s.id} className={`step is-${status}`} aria-current={status === "current" ? "step" : undefined}>
                <span className="step-num">{status === "done" ? <Icon name="check" size={12} /> : `0${i + 1}`}</span>
                <div className="step-text">
                  <span className="step-title">{s.title}</span>
                  {status === "current" ? <span className="step-body">{s.body}</span> : null}
                  {status === "current" && s.id === "connectAgent37" ? (
                    <form
                      className="key-form"
                      onSubmit={(e) => {
                        e.preventDefault();
                        if (looksLikeKey && !busy) onSaveKey(key.trim()).then(() => setKey(""));
                      }}
                    >
                      <input
                        className="key-input"
                        type="password"
                        autoComplete="off"
                        spellCheck={false}
                        placeholder="sk_live_…"
                        value={key}
                        onChange={(e) => setKey(e.target.value)}
                        disabled={busy}
                        aria-label="Agent37 API key"
                      />
                      <div className="key-actions">
                        <button className="btn btn-solid btn-lg" type="submit" disabled={busy || !looksLikeKey} aria-busy={busy}>
                          {busy ? (
                            <>
                              <Dots /> Checking key
                            </>
                          ) : (
                            "Connect"
                          )}
                        </button>
                        <button className="btn btn-link" type="button" disabled={busy} onClick={() => onAction("openKeys")}>
                          Get a key
                        </button>
                      </div>
                    </form>
                  ) : null}
                  {status === "current" && s.id === "installT3" ? (
                    <button className="btn btn-solid btn-lg" disabled={busy} onClick={() => onAction("installT3")}>
                      Install T3 Code
                    </button>
                  ) : null}
                </div>
              </li>
            );
          })}
        </ol>
      </div>
      <div className="view-foot">
        <button className="btn btn-link" disabled={busy} onClick={() => onAction("recheck")}>
          Done it already? Check again
        </button>
      </div>
    </div>
  );
}
