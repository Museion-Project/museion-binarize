import { useState } from "react";

import type { DocumentSummary } from "../app/types";

interface DocumentInfoProps {
  document: DocumentSummary;
  currentPage: number;
}

/** Compact document metadata, with PDFium provenance tucked behind a
 * disclosure so the main view stays uncluttered — see docs/desktop.md,
 * "Document summary". */
export function DocumentInfo({ document, currentPage }: DocumentInfoProps) {
  const [expanded, setExpanded] = useState(false);
  const page = document.pages.find((p) => p.pageNumber === currentPage);

  return (
    <div className="document-info">
      {(document.title || document.author) && (
        <p className="document-info-title">
          {document.title ?? document.fileName}
          {document.author && <span className="document-info-author"> — {document.author}</span>}
        </p>
      )}
      {page && (
        <p className="document-info-page">
          Page {page.pageNumber}: {page.widthPoints.toFixed(0)} × {page.heightPoints.toFixed(0)} pt
          {page.sourceRotationDegrees !== 0 && ` · source rotation ${page.sourceRotationDegrees}°`}
        </p>
      )}
      <button
        type="button"
        className="document-info-toggle"
        onClick={() => setExpanded((v) => !v)}
        aria-expanded={expanded}
      >
        {expanded ? "Hide details" : "Details"}
      </button>
      {expanded && (
        <dl className="document-info-details">
          <dt>PDFium</dt>
          <dd>{document.pdfiumLibrary}</dd>
        </dl>
      )}
    </div>
  );
}
