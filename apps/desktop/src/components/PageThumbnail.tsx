import { useEffect, useRef } from "react";

interface PageThumbnailProps {
  pageNumber: number;
  selected: boolean;
  dataUrl: string | null;
  onVisible: (pageNumber: number) => void;
  onSelect: (pageNumber: number) => void;
}

/** One thumbnail in the sidebar. Reports visibility via IntersectionObserver
 * so the parent can lazily fetch its image only once it actually scrolls
 * into view — see docs/desktop.md, "Thumbnail sidebar". */
export function PageThumbnail({
  pageNumber,
  selected,
  dataUrl,
  onVisible,
  onSelect,
}: PageThumbnailProps) {
  const ref = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    const node = ref.current;
    if (!node || dataUrl) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          onVisible(pageNumber);
        }
      },
      { root: node.closest(".page-sidebar-list"), rootMargin: "200px 0px" },
    );
    observer.observe(node);
    return () => observer.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pageNumber, dataUrl]);

  useEffect(() => {
    if (selected) {
      ref.current?.scrollIntoView({ block: "nearest" });
    }
  }, [selected]);

  return (
    <button
      ref={ref}
      type="button"
      className={`page-thumbnail${selected ? " selected" : ""}`}
      aria-current={selected ? "true" : undefined}
      aria-label={`Page ${pageNumber}`}
      onClick={() => onSelect(pageNumber)}
    >
      <span className="page-thumbnail-image">
        {dataUrl ? <img src={dataUrl} alt="" /> : <span className="page-thumbnail-placeholder" />}
      </span>
      <span className="page-thumbnail-number">{pageNumber}</span>
    </button>
  );
}
