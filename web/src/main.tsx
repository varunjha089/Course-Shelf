import { Component, StrictMode, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "./styles.css";

// A render error used to leave a blank window. Show it instead.
class Boundary extends Component<{ children: ReactNode }, { err: Error | null }> {
  state = { err: null as Error | null };
  static getDerivedStateFromError(err: Error) { return { err }; }
  render() {
    if (!this.state.err) return this.props.children;
    return (
      <div style={{ padding: 24 }}>
        <h2>Something went wrong on this page</h2>
        <pre style={{ whiteSpace: "pre-wrap" }}>{String(this.state.err.stack ?? this.state.err)}</pre>
        <button className="btn" onClick={() => { location.hash = "#/"; this.setState({ err: null }); }}>Back to start</button>
      </div>
    );
  }
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Boundary>
      <App />
    </Boundary>
  </StrictMode>,
);
