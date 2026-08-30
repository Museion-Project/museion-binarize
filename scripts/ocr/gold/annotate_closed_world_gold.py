#!/usr/bin/env python3
"""Small local macOS GUI for creating closed-world OCR gold.

The tool never calls a model. CGPG boxes/text are orange, unverified
scaffolding. Green lines are human verified. Shift-drag adds a missing line.
"""

from __future__ import annotations

import argparse
import copy
import sys
import unicodedata
from pathlib import Path

from PIL import Image, ImageTk

sys.path.insert(0, str(Path(__file__).resolve().parent))

import cgpg  # noqa: E402
import closed_world_gold as gold  # noqa: E402


REPO_ROOT = Path(__file__).resolve().parents[3]
DEFAULT_OUTPUT = REPO_ROOT / "gold-data/closed-world-cgpg-v1/pages"


class GoldAnnotator:
    CANVAS_WIDTH = 650
    CANVAS_HEIGHT = 760

    def __init__(self, root, corpus_root: Path, output_root: Path, page_ids: list[str]):
        import tkinter as tk
        from tkinter import ttk

        self.tk = tk
        self.ttk = ttk
        self.root = root
        self.corpus_root = corpus_root
        self.output_root = output_root
        self.page_ids = page_ids
        self.page_position = 0
        self.page = None
        self.record = None
        self.image = None
        self.display_photo = None
        self.zoom_photo = None
        self.scale = 1.0
        self.selected = None
        self.drag_start = None
        self.drag_rectangle = None
        self.loading_form = False

        root.title("M PDF · Closed-world Gold")
        root.geometry("1250x850")
        root.minsize(1060, 720)
        root.protocol("WM_DELETE_WINDOW", self.close)

        outer = ttk.Frame(root, padding=8)
        outer.pack(fill="both", expand=True)
        toolbar = ttk.Frame(outer)
        toolbar.pack(fill="x", pady=(0, 6))
        ttk.Button(toolbar, text="◀ 上一页", command=self.previous_page).pack(side="left")
        ttk.Button(toolbar, text="下一页 ▶", command=self.next_page).pack(side="left", padx=5)
        self.page_label = ttk.Label(toolbar, text="")
        self.page_label.pack(side="left", padx=12)
        ttk.Label(
            toolbar,
            text="单击选框 · Shift+拖动补框 · 橙色=候选 · 绿色=人工核验",
        ).pack(side="right")

        body = ttk.Panedwindow(outer, orient="horizontal")
        body.pack(fill="both", expand=True)
        left = ttk.Frame(body)
        right = ttk.Frame(body, padding=(10, 0, 0, 0))
        body.add(left, weight=3)
        body.add(right, weight=2)

        self.canvas = tk.Canvas(
            left,
            width=self.CANVAS_WIDTH,
            height=self.CANVAS_HEIGHT,
            background="#252525",
            highlightthickness=0,
        )
        self.canvas.pack(fill="both", expand=True)
        self.canvas.bind("<Button-1>", self.select_at)
        self.canvas.bind("<Shift-ButtonPress-1>", self.start_new_box)
        self.canvas.bind("<Shift-B1-Motion>", self.drag_new_box)
        self.canvas.bind("<Shift-ButtonRelease-1>", self.finish_new_box)

        self.line_label = ttk.Label(right, text="未选择行")
        self.line_label.pack(fill="x")
        self.status_label = ttk.Label(right, text="")
        self.status_label.pack(fill="x", pady=(2, 5))
        self.listbox = tk.Listbox(right, height=10, exportselection=False)
        self.listbox.pack(fill="x")
        self.listbox.bind("<<ListboxSelect>>", self.select_from_list)

        self.zoom = tk.Canvas(right, height=115, background="#111", highlightthickness=0)
        self.zoom.pack(fill="x", pady=6)

        ttk.Label(right, text="逐字转录（保持原样）").pack(anchor="w")
        self.text = tk.Text(right, height=4, wrap="word", undo=True)
        self.text.pack(fill="x")

        labels = ttk.Frame(right)
        labels.pack(fill="x", pady=6)
        ttk.Label(labels, text="语言").grid(row=0, column=0, sticky="w")
        self.language = tk.StringVar(value="grc")
        ttk.Combobox(
            labels, textvariable=self.language, values=gold.LANGUAGES, state="readonly", width=10
        ).grid(row=0, column=1, padx=(4, 12))
        ttk.Label(labels, text="类别").grid(row=0, column=2, sticky="w")
        self.content_class = tk.StringVar(value="unclassified")
        ttk.Combobox(
            labels,
            textvariable=self.content_class,
            values=gold.CONTENT_CLASSES,
            state="readonly",
            width=16,
        ).grid(row=0, column=3, padx=4)

        box_frame = ttk.LabelFrame(right, text="框坐标：left, top, right, bottom", padding=5)
        box_frame.pack(fill="x")
        self.box_vars = [tk.StringVar() for _ in range(4)]
        for index, variable in enumerate(self.box_vars):
            ttk.Entry(box_frame, textvariable=variable, width=8).grid(row=0, column=index, padx=2)
        ttk.Button(box_frame, text="应用框", command=self.apply_box).grid(row=0, column=4, padx=5)

        row = ttk.Frame(right)
        row.pack(fill="x", pady=6)
        ttk.Button(row, text="✓ 核验此行", command=self.verify_line).pack(side="left")
        ttk.Button(row, text="↑", width=3, command=lambda: self.move_line(-1)).pack(side="left", padx=(8, 2))
        ttk.Button(row, text="↓", width=3, command=lambda: self.move_line(1)).pack(side="left")
        ttk.Button(row, text="删除行", command=self.delete_line).pack(side="right")

        coverage = ttk.LabelFrame(right, text="页面覆盖门禁", padding=6)
        coverage.pack(fill="x", pady=(4, 0))
        self.exhaustive = tk.BooleanVar(value=False)
        ttk.Checkbutton(
            coverage,
            text="我已检查整页，所有可见文字行均已包含",
            variable=self.exhaustive,
        ).pack(anchor="w")
        reviewer_row = ttk.Frame(coverage)
        reviewer_row.pack(fill="x", pady=4)
        ttk.Label(reviewer_row, text="核验人").pack(side="left")
        self.reviewer = tk.StringVar()
        ttk.Entry(reviewer_row, textvariable=self.reviewer).pack(side="left", fill="x", expand=True, padx=5)
        ttk.Label(coverage, text="未解决问题（必须清空才可完成）").pack(anchor="w")
        self.notes = tk.Text(coverage, height=2, wrap="word")
        self.notes.pack(fill="x")

        actions = ttk.Frame(right)
        actions.pack(fill="x", pady=8)
        ttk.Button(actions, text="保存草稿  ⌘S", command=self.save_draft).pack(side="left")
        ttk.Button(actions, text="完成并核验页面", command=self.verify_page).pack(side="right")

        root.bind("<Command-s>", lambda _event: self.save_draft())
        root.bind("<Control-s>", lambda _event: self.save_draft())
        root.bind("<Command-Return>", lambda _event: self.verify_line())
        self.load_page(0)
        root.lift()
        root.attributes("-topmost", True)
        root.after(600, lambda: root.attributes("-topmost", False))

    def record_path(self, page_id: str | None = None) -> Path:
        return self.output_root / f"{page_id or self.page.name}.json"

    def load_page(self, position: int) -> None:
        from tkinter import messagebox

        page_id = self.page_ids[position]
        try:
            page = cgpg.load_page(self.corpus_root / f"{page_id}.xml")
        except Exception as error:
            messagebox.showerror("无法加载页面", str(error))
            return
        self.page_position = position
        self.page = page
        path = self.record_path(page_id)
        self.record = gold.load_record(path) if path.exists() else gold.draft_from_cgpg(page)
        try:
            gold.validate_record(self.record, require_complete=False, image_path=page.image_path)
        except gold.GoldError as error:
            messagebox.showerror("草稿损坏或图片不匹配", str(error))
            return
        self.image = Image.open(page.image_path).convert("RGB")
        self.selected = 0 if self.record["lines"] else None
        coverage = self.record["coverage"]
        self.exhaustive.set(bool(coverage["all_visible_lines_exhaustively_reviewed"]))
        self.reviewer.set(coverage["reviewer"])
        self.notes.delete("1.0", "end")
        self.notes.insert("1.0", coverage["unresolved_notes"])
        self.page_label.configure(
            text=f"{position + 1}/{len(self.page_ids)}  {page_id}  ·  {page.width}×{page.height}"
        )
        self.render()
        self.load_form()

    def render(self) -> None:
        canvas_width = max(self.canvas.winfo_width(), self.CANVAS_WIDTH)
        canvas_height = max(self.canvas.winfo_height(), self.CANVAS_HEIGHT)
        self.scale = min(canvas_width / self.page.width, canvas_height / self.page.height)
        size = (max(1, int(self.page.width * self.scale)), max(1, int(self.page.height * self.scale)))
        display = self.image.resize(size, Image.Resampling.LANCZOS)
        self.display_photo = ImageTk.PhotoImage(display)
        self.canvas.delete("all")
        self.canvas.create_image(0, 0, image=self.display_photo, anchor="nw")
        for index, line in enumerate(self.record["lines"]):
            left, top, right, bottom = [value * self.scale for value in line["bbox"]]
            verified = (
                line["geometry_status"] == "human_verified"
                and line["transcription_status"] == "human_verified"
            )
            color = "#4ee06f" if verified else "#ffad33"
            width = 3 if index == self.selected else 1
            if index == self.selected:
                color = "#ff3b5c"
            self.canvas.create_rectangle(left, top, right, bottom, outline=color, width=width)
        self.refresh_list()
        self.refresh_status()

    def refresh_list(self) -> None:
        self.listbox.delete(0, "end")
        for index, line in enumerate(self.record["lines"]):
            verified = line["geometry_status"] == line["transcription_status"] == "human_verified"
            text = line["text"].replace("\n", " ")
            self.listbox.insert("end", f"{'✓' if verified else '○'} {index + 1:03d}  {text[:48]}")
        if self.selected is not None and self.selected < self.listbox.size():
            self.listbox.selection_set(self.selected)
            self.listbox.see(self.selected)

    def refresh_status(self) -> None:
        verified = sum(
            line["geometry_status"] == line["transcription_status"] == "human_verified"
            for line in self.record["lines"]
        )
        self.status_label.configure(
            text=f"已核验 {verified}/{len(self.record['lines'])} 行 · 页面状态 {self.record['coverage']['status']}"
        )

    def select_from_list(self, _event=None) -> None:
        selection = self.listbox.curselection()
        if selection:
            self.select_line(selection[0])

    def select_at(self, event) -> None:
        if event.state & 0x0001:
            return
        x, y = event.x / self.scale, event.y / self.scale
        candidates = []
        for index, line in enumerate(self.record["lines"]):
            left, top, right, bottom = line["bbox"]
            if left <= x <= right and top <= y <= bottom:
                candidates.append(((right - left) * (bottom - top), index))
        if candidates:
            self.select_line(min(candidates)[1])

    def select_line(self, index: int) -> None:
        if self.selected == index:
            return
        self.commit_form()
        self.selected = index
        self.render()
        self.load_form()

    def load_form(self) -> None:
        self.loading_form = True
        if self.selected is None or self.selected >= len(self.record["lines"]):
            self.line_label.configure(text="未选择行")
            self.text.delete("1.0", "end")
            self.zoom.delete("all")
            self.loading_form = False
            return
        line = self.record["lines"][self.selected]
        self.line_label.configure(text=f"行 {self.selected + 1} · {line['line_id']}")
        self.text.delete("1.0", "end")
        self.text.insert("1.0", line["text"])
        self.language.set(line["language"])
        self.content_class.set(line["content_class"])
        for variable, value in zip(self.box_vars, line["bbox"]):
            variable.set(str(int(value) if float(value).is_integer() else value))
        self.render_zoom(line["bbox"])
        self.loading_form = False

    def render_zoom(self, bbox) -> None:
        left, top, right, bottom = map(int, bbox)
        padding = max(8, int((bottom - top) * 0.8))
        crop = self.image.crop(
            (max(0, left - padding), max(0, top - padding), min(self.page.width, right + padding), min(self.page.height, bottom + padding))
        )
        width = max(self.zoom.winfo_width(), 480)
        height = max(self.zoom.winfo_height(), 115)
        factor = min(width / crop.width, height / crop.height)
        crop = crop.resize((max(1, int(crop.width * factor)), max(1, int(crop.height * factor))), Image.Resampling.LANCZOS)
        self.zoom_photo = ImageTk.PhotoImage(crop)
        self.zoom.delete("all")
        self.zoom.create_image(width // 2, height // 2, image=self.zoom_photo, anchor="center")

    def invalidate_page(self) -> None:
        coverage = self.record["coverage"]
        coverage["status"] = "draft"
        coverage["verified_at"] = None

    def commit_form(self) -> None:
        if self.loading_form or self.selected is None or self.selected >= len(self.record["lines"]):
            return
        line = self.record["lines"][self.selected]
        proposed = {
            "text": unicodedata.normalize("NFC", self.text.get("1.0", "end-1c")),
            "language": self.language.get(),
            "content_class": self.content_class.get(),
        }
        if any(line[key] != value for key, value in proposed.items()):
            line.update(proposed)
            line["transcription_status"] = "candidate_unverified"
            line["source"] = "human_draft"
            self.invalidate_page()

    def apply_box(self) -> None:
        from tkinter import messagebox

        if self.selected is None:
            return
        try:
            bbox = [float(variable.get()) for variable in self.box_vars]
            left, top, right, bottom = bbox
            if not (0 <= left < right <= self.page.width and 0 <= top < bottom <= self.page.height):
                raise ValueError
        except ValueError:
            messagebox.showerror("框无效", "坐标必须位于页面内，且 left < right、top < bottom。")
            return
        line = self.record["lines"][self.selected]
        if line["bbox"] != bbox:
            line["bbox"] = bbox
            line["geometry_status"] = "candidate_unverified"
            line["source"] = "human_draft"
            self.invalidate_page()
        self.render()
        self.load_form()

    def verify_line(self) -> None:
        from tkinter import messagebox

        if self.selected is None:
            return
        self.commit_form()
        line = self.record["lines"][self.selected]
        if not line["text"].strip():
            messagebox.showerror("不能核验", "文字不能为空；无法辨认的单字请填 �。")
            return
        if line["content_class"] == "unclassified":
            messagebox.showerror("不能核验", "请先选择行类别。")
            return
        line["geometry_status"] = "human_verified"
        line["transcription_status"] = "human_verified"
        line["source"] = "human"
        self.invalidate_page()
        self.render()
        if self.selected + 1 < len(self.record["lines"]):
            self.select_line(self.selected + 1)
        else:
            self.load_form()

    def start_new_box(self, event) -> None:
        self.drag_start = (event.x, event.y)
        self.drag_rectangle = self.canvas.create_rectangle(event.x, event.y, event.x, event.y, outline="#4da3ff", width=2)

    def drag_new_box(self, event) -> None:
        if self.drag_start and self.drag_rectangle:
            self.canvas.coords(self.drag_rectangle, self.drag_start[0], self.drag_start[1], event.x, event.y)

    def finish_new_box(self, event) -> None:
        if not self.drag_start:
            return
        x1, y1 = self.drag_start
        self.drag_start = None
        if abs(event.x - x1) < 4 or abs(event.y - y1) < 4:
            self.render()
            return
        self.commit_form()
        left, right = sorted((x1 / self.scale, event.x / self.scale))
        top, bottom = sorted((y1 / self.scale, event.y / self.scale))
        left, top = max(0, left), max(0, top)
        right, bottom = min(self.page.width, right), min(self.page.height, bottom)
        used = {line["line_id"] for line in self.record["lines"]}
        counter = len(used)
        while f"{self.page.name}-human-{counter:04d}" in used:
            counter += 1
        self.record["lines"].append(
            {
                "line_id": f"{self.page.name}-human-{counter:04d}",
                "reading_order": len(self.record["lines"]),
                "bbox": [round(left, 2), round(top, 2), round(right, 2), round(bottom, 2)],
                "text": "",
                "language": "grc",
                "content_class": "unclassified",
                "geometry_status": "candidate_unverified",
                "transcription_status": "candidate_unverified",
                "source": "human_draft",
            }
        )
        self.invalidate_page()
        self.selected = len(self.record["lines"]) - 1
        self.render()
        self.load_form()
        self.text.focus_set()

    def renumber(self) -> None:
        for order, line in enumerate(self.record["lines"]):
            line["reading_order"] = order

    def move_line(self, direction: int) -> None:
        if self.selected is None:
            return
        target = self.selected + direction
        if not 0 <= target < len(self.record["lines"]):
            return
        self.commit_form()
        self.record["lines"][self.selected], self.record["lines"][target] = self.record["lines"][target], self.record["lines"][self.selected]
        self.selected = target
        self.renumber()
        self.invalidate_page()
        self.render()
        self.load_form()

    def delete_line(self) -> None:
        from tkinter import messagebox

        if self.selected is None or not messagebox.askyesno("删除行", "确定删除当前行框？草稿保存前仍可关闭窗口放弃。"):
            return
        del self.record["lines"][self.selected]
        self.renumber()
        self.invalidate_page()
        self.selected = min(self.selected, len(self.record["lines"]) - 1) if self.record["lines"] else None
        self.render()
        self.load_form()

    def update_coverage_form(self) -> None:
        coverage = self.record["coverage"]
        coverage["all_visible_lines_exhaustively_reviewed"] = bool(self.exhaustive.get())
        coverage["reviewer"] = self.reviewer.get().strip()
        coverage["unresolved_notes"] = self.notes.get("1.0", "end-1c")

    def save_draft(self, quiet: bool = False) -> bool:
        from tkinter import messagebox

        self.commit_form()
        self.update_coverage_form()
        try:
            gold.validate_record(self.record, require_complete=False, image_path=self.page.image_path)
            gold.atomic_write_json(self.record_path(), self.record)
        except (gold.GoldError, OSError) as error:
            messagebox.showerror("保存失败", str(error))
            return False
        if not quiet:
            messagebox.showinfo("已保存", f"草稿已原子写入：\n{self.record_path()}")
        self.refresh_status()
        return True

    def verify_page(self) -> None:
        from tkinter import messagebox

        self.commit_form()
        self.update_coverage_form()
        try:
            gold.mark_verified(self.record, self.reviewer.get())
            gold.validate_record(self.record, require_complete=True, image_path=self.page.image_path)
            gold.atomic_write_json(self.record_path(), self.record)
        except (gold.GoldError, OSError) as error:
            messagebox.showerror("页面尚不能完成", str(error))
            return
        self.render()
        messagebox.showinfo("页面已核验", "此页已通过 closed-world coverage contract。")

    def previous_page(self) -> None:
        if self.page_position == 0:
            return
        if self.save_draft(quiet=True):
            self.load_page(self.page_position - 1)

    def next_page(self) -> None:
        if self.page_position + 1 >= len(self.page_ids):
            return
        if self.save_draft(quiet=True):
            self.load_page(self.page_position + 1)

    def close(self) -> None:
        from tkinter import messagebox

        if messagebox.askyesno("保存并退出", "保存当前草稿并关闭窗口？"):
            if self.save_draft(quiet=True):
                self.root.destroy()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus-root", required=True, type=Path)
    parser.add_argument("--output-root", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--all-holdout", action="store_true", help="annotate all 12 frozen holdout pages")
    parser.add_argument("--pages", nargs="+", choices=gold.HOLDOUT_PAGE_IDS)
    return parser.parse_args()


def main() -> int:
    import tkinter as tk

    args = parse_args()
    page_ids = list(args.pages or (gold.HOLDOUT_PAGE_IDS if args.all_holdout else gold.FORENSIC_PAGE_IDS))
    missing = [page_id for page_id in page_ids if not (args.corpus_root / f"{page_id}.xml").is_file()]
    if missing:
        raise SystemExit(f"missing corpus pages: {', '.join(missing)}")
    root = tk.Tk()
    GoldAnnotator(root, args.corpus_root.resolve(), args.output_root.resolve(), page_ids)
    root.mainloop()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
