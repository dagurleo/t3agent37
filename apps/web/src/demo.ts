// The panel (src/panel, the React port of the app's panel) on its mock backend: sample machines,
// nothing real. The site around this frame plays T3 Code on that backend (see Together.tsx).
import "./panel/main.tsx";
import "./demo.css";
import type { MockBackend } from "./panel/mock";

declare global {
  interface Window {
    mock?: Promise<MockBackend>;
  }
}

window.mock = import("./panel/mock").then((m) => m.mockBackend());
