import { useCallback, useEffect, useRef, useState } from "react";
import type { CommandArgs, CommandName, CommandResult, MachineDetails, UiState } from "./types";

export interface Backend {
  call<C extends CommandName>(cmd: C, args: CommandArgs<C>): Promise<CommandResult<C>>;
  /** Calls `cb` on every "state" event. Returns an unsubscribe function. */
  subscribe(cb: (s: UiState) => void): () => void;
}

// The site runs the panel on its mock backend only: sample machines, nothing real. (In the app,
// before it went native, this picked Tauri's IPC when it ran inside Tauri.)
let backendPromise: Promise<Backend> | null = null;
function backend(): Promise<Backend> {
  backendPromise ??= import("./mock").then((m) => m.mockBackend());
  return backendPromise;
}

export function call<C extends CommandName>(cmd: C, args: CommandArgs<C>): Promise<CommandResult<C>> {
  return backend().then((b) => b.call(cmd, args));
}

/** Rejections are strings per contract; normalise anything else. */
export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return "Something went wrong";
}

export function useUiState(): { state: UiState | null; loadError: string | null; retry: () => void } {
  const [state, setState] = useState<UiState | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let alive = true;
    let unsubscribe: (() => void) | undefined;
    backend().then((b) => {
      if (!alive) return;
      unsubscribe = b.subscribe((s) => alive && setState(s));
      b.call("get_state", undefined).then(
        (s) => alive && setState(s),
        (e) => alive && setLoadError(errorText(e)),
      );
    });
    return () => {
      alive = false;
      unsubscribe?.();
    };
  }, [attempt]);

  const retry = useCallback(() => {
    setLoadError(null);
    setAttempt((n) => n + 1);
  }, []);

  return { state, loadError, retry };
}

/**
 * Fetches a machine's details now and again whenever `version` changes (pass the
 * latest UiState: a new object arrives with every "state" event). A failed refetch
 * keeps the last data; `onError` fires once per run of failures.
 */
export function useMachineDetails(id: string, version: unknown, onError: (text: string) => void): MachineDetails | null {
  const [details, setDetails] = useState<MachineDetails | null>(null);
  const failing = useRef(false);
  const latest = useRef(0);

  useEffect(() => {
    const req = ++latest.current;
    call("get_machine_details", { id }).then(
      (d) => {
        if (req !== latest.current) return;
        failing.current = false;
        setDetails(d);
      },
      (e) => {
        if (req !== latest.current || failing.current) return;
        failing.current = true;
        onError(errorText(e));
      },
    );
  }, [id, version, onError]);

  return details?.id === id ? details : null;
}

/** The machine's latest thread id, looked up once per machine; null when it has none. */
export function useLatestThread(id: string): string | null {
  const [latest, setLatest] = useState<{ id: string; thread: string | null } | null>(null);
  useEffect(() => {
    let live = true;
    call("latest_thread", { id }).then(
      (thread) => live && setLatest({ id, thread }),
      () => live && setLatest({ id, thread: null }),
    );
    return () => {
      live = false;
    };
  }, [id]);
  return latest?.id === id ? latest.thread : null;
}
