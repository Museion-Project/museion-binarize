import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

interface ProjectInfo {
  name: string;
  phase: string;
}

function App() {
  const [projectInfo, setProjectInfo] = useState<ProjectInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<ProjectInfo>("project_info")
      .then(setProjectInfo)
      .catch((err) => setError(String(err)));
  }, []);

  return (
    <main className="container">
      <h1>{projectInfo?.name ?? "Museion Binarize"}</h1>
      <p className="phase-badge">{projectInfo?.phase ?? "Phase 1 — under development"}</p>

      <p>
        All processing runs locally on your machine. No scan, page image, or
        output file is uploaded to any network service.
      </p>

      <button type="button" disabled title="PDF conversion is not implemented yet">
        Open PDF (coming soon)
      </button>

      {error && <p className="bridge-error">Backend bridge error: {error}</p>}

      <p className="docs-links">
        Learn more:{" "}
        <a
          href="https://github.com/Museion-Project/museion-binarize/blob/main/docs/roadmap.md"
          target="_blank"
          rel="noreferrer"
        >
          roadmap
        </a>{" "}
        ·{" "}
        <a
          href="https://github.com/Museion-Project/museion-binarize/blob/main/docs/limitations.md"
          target="_blank"
          rel="noreferrer"
        >
          current limitations
        </a>
      </p>
    </main>
  );
}

export default App;
