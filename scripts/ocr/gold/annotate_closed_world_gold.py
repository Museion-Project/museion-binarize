#!/usr/bin/env python3
"""Small local macOS GUI for creating closed-world OCR gold.

The default mode opens arbitrary page images and starts from a blank annotation;
the optional CGPG mode keeps its boxes/text as orange, unverified scaffolding.
The tool never calls a model. Green lines are human verified. Shift-drag adds a
line.
"""

from __future__ import annotations

import argparse
import sys
import unicodedata
from dataclasses import dataclass
from pathlib import Path

from PIL import Image, ImageTk

sys.path.insert(0, str(Path(__file__).resolve().parent))

import cgpg  # noqa: E402
import closed_world_gold as gold  # noqa: E402
import annotator_workflow as workflow  # noqa: E402


REPO_ROOT = Path(__file__).resolve().parents[3]
DEFAULT_OUTPUT = REPO_ROOT / "gold-data/mixed-scholarly-v1/pages"
CGPG_OUTPUT = REPO_ROOT / "gold-data/closed-world-cgpg-v1/pages"


@dataclass(frozen=True)
class ImagePage:
    name: str
    image_path: Path
    width: int
    height: int


def load_image_page(path: Path) -> ImagePage:
    path = path.resolve()
    with Image.open(path) as image:
        width, height = image.size
    return ImagePage(path.stem, path, width, height)


class GoldAnnotator:
    CANVAS_WIDTH = 650
    CANVAS_HEIGHT = 760

    def __init__(
        self, root, pages: list[object], output_root: Path, *, cgpg_mode: bool, review_scope: str = "full-gold"
    ):
        import tkinter as tk
        from tkinter import ttk

        self.tk = tk
        self.ttk = ttk
        self.root = root
        self.pages = pages
        self.output_root = output_root
        self.page_ids = [page.name for page in pages]
        self.cgpg_mode = cgpg_mode
        self.review_scope = review_scope
        self.disk_digest = None
        self.default_language = "grc" if cgpg_mode else "mixed"
        self.page_position = 0
        self.page = None
        self.record = None
        self.image = None
        self.display_photo = None
        self.zoom_photo = None
        self.scale = 1.0
        self.zoom_factor = 1.0
        self.selected = None
        self.drag_start = None
        self.drag_rectangle = None
        self.loading_form = False

        root.title("M PDF · Dev reference QA" if review_scope == "ocr-geometry" else "M PDF · Mixed-script Closed-world Gold")
        root.geometry("1250x850")
        root.minsize(1060, 720)
        root.protocol("WM_DELETE_WINDOW", self.close)

        outer = ttk.Frame(root, padding=8)
        outer.pack(fill="both", expand=True)
        toolbar = ttk.Frame(outer)
        toolbar.pack(fill="x", pady=(0, 6))
        ttk.Button(toolbar, text="◀ 上一页", command=self.previous_page).pack(
            side="left"
        )
        ttk.Button(toolbar, text="下一页 ▶", command=self.next_page).pack(
            side="left", padx=5
        )
        self.page_label = ttk.Label(toolbar, text="")
        self.page_label.pack(side="left", padx=12)
        ttk.Label(
            toolbar,
            text=(
                "单击选框 · Shift+拖动加框 · "
                "橙=未核验 · 蓝=几何已核验 · 绿=几何+字符已核验"
            ),
        ).pack(side="right")

        body = ttk.Panedwindow(outer, orient="horizontal")
        body.pack(fill="both", expand=True)
        left = ttk.Frame(body)
        right = ttk.Frame(body, padding=(10, 0, 0, 0))
        body.add(left, weight=3)
        body.add(right, weight=2)

        zoom_bar = ttk.Frame(left)
        zoom_bar.pack(fill="x", pady=(0, 4))
        ttk.Button(zoom_bar, text="−", width=3, command=self.zoom_out).pack(
            side="left"
        )
        ttk.Button(zoom_bar, text="适应", command=self.reset_zoom).pack(
            side="left", padx=4
        )
        ttk.Button(zoom_bar, text="+", width=3, command=self.zoom_in).pack(
            side="left"
        )
        self.canvas_zoom_label = ttk.Label(zoom_bar, text="100%")
        self.canvas_zoom_label.pack(side="left", padx=8)
        ttk.Label(
            zoom_bar,
            text="⌘/⌃+滚轮缩放 · 滚动条平移",
        ).pack(side="right")

        canvas_frame = ttk.Frame(left)
        canvas_frame.pack(fill="both", expand=True)
        self.canvas = tk.Canvas(
            canvas_frame,
            width=self.CANVAS_WIDTH,
            height=self.CANVAS_HEIGHT,
            background="#252525",
            highlightthickness=0,
        )
        horizontal = ttk.Scrollbar(
            canvas_frame, orient="horizontal", command=self.canvas.xview
        )
        vertical = ttk.Scrollbar(
            canvas_frame, orient="vertical", command=self.canvas.yview
        )
        self.canvas.configure(
            xscrollcommand=horizontal.set,
            yscrollcommand=vertical.set,
        )
        self.canvas.grid(row=0, column=0, sticky="nsew")
        vertical.grid(row=0, column=1, sticky="ns")
        horizontal.grid(row=1, column=0, sticky="ew")
        canvas_frame.rowconfigure(0, weight=1)
        canvas_frame.columnconfigure(0, weight=1)
        self.canvas.bind("<Button-1>", self.select_at)
        self.canvas.bind("<Shift-ButtonPress-1>", self.start_new_box)
        self.canvas.bind("<Shift-B1-Motion>", self.drag_new_box)
        self.canvas.bind("<Shift-ButtonRelease-1>", self.finish_new_box)
        self.canvas.bind("<Command-MouseWheel>", self.zoom_with_wheel)
        self.canvas.bind("<Control-MouseWheel>", self.zoom_with_wheel)

        self.line_label = ttk.Label(right, text="未选择行")
        self.line_label.pack(fill="x")
        self.status_label = ttk.Label(right, text="")
        self.status_label.pack(fill="x", pady=(2, 5))
        self.listbox = tk.Listbox(right, height=8, exportselection=False)
        self.listbox.pack(fill="x")
        self.listbox.bind("<<ListboxSelect>>", self.select_from_list)

        self.zoom = tk.Canvas(right, height=88, background="#111", highlightthickness=0)
        self.zoom.pack(fill="x", pady=6)

        modules = ttk.Notebook(right)
        modules.pack(fill="both", expand=True)
        geometry_tab = ttk.Frame(modules, padding=6)
        character_tab = ttk.Frame(modules, padding=6)
        page_tab = ttk.Frame(modules, padding=6)
        modules.add(geometry_tab, text="1. 几何（D 延期）" if self.review_scope == "ocr-geometry" else "1. 几何 + 结构")
        modules.add(character_tab, text="2. 字符（D 延期）" if self.review_scope == "ocr-geometry" else "2. 字符 + 排印")
        modules.add(page_tab, text="3. 页面门禁")

        geometry_module = ttk.LabelFrame(
            geometry_tab,
            text="1. 几何模块 · 行框与阅读顺序",
            padding=6,
        )
        geometry_module.pack(fill="x", pady=(0, 6))
        box_frame = ttk.Frame(geometry_module)
        box_frame.pack(fill="x")
        ttk.Label(box_frame, text="left, top, right, bottom").grid(
            row=0, column=0, columnspan=4, sticky="w"
        )
        self.box_vars = [tk.StringVar() for _ in range(4)]
        for index, variable in enumerate(self.box_vars):
            ttk.Entry(box_frame, textvariable=variable, width=8).grid(
                row=1, column=index, padx=2
            )
        ttk.Button(box_frame, text="应用框", command=self.apply_box).grid(
            row=1, column=4, padx=5
        )

        geometry_actions = ttk.Frame(geometry_module)
        geometry_actions.pack(fill="x", pady=(6, 0))
        ttk.Button(
            geometry_actions,
            text="✓ 核验几何",
            command=self.verify_geometry,
        ).pack(side="left")
        ttk.Button(
            geometry_actions,
            text="↑",
            width=3,
            command=lambda: self.move_line(-1),
        ).pack(side="left", padx=(8, 2))
        ttk.Button(
            geometry_actions,
            text="↓",
            width=3,
            command=lambda: self.move_line(1),
        ).pack(side="left")
        ttk.Button(
            geometry_actions,
            text="删除行",
            command=self.delete_line,
        ).pack(side="right")

        structure = ttk.Frame(geometry_module)
        structure.pack(fill="x", pady=(6, 0))
        ttk.Label(structure, text="段落").grid(row=0, column=0, sticky="w")
        self.paragraph_role = tk.StringVar(value="unclassified")
        ttk.Combobox(
            structure,
            textvariable=self.paragraph_role,
            values=gold.PARAGRAPH_ROLES,
            state="readonly",
            width=13,
        ).grid(row=0, column=1, padx=(3, 8))
        ttk.Label(structure, text="叶面").grid(row=0, column=2, sticky="w")
        self.leaf_id = tk.StringVar()
        ttk.Entry(structure, textvariable=self.leaf_id, width=12).grid(
            row=0, column=3, padx=(3, 8)
        )
        ttk.Label(structure, text="栏位").grid(row=0, column=4, sticky="w")
        self.column_id = tk.StringVar()
        ttk.Entry(structure, textvariable=self.column_id, width=12).grid(
            row=0, column=5, padx=3
        )
        ttk.Label(structure, text="目录层级").grid(
            row=1, column=0, sticky="w", pady=(4, 0)
        )
        self.hierarchy_level = tk.StringVar()
        ttk.Entry(structure, textvariable=self.hierarchy_level, width=6).grid(
            row=1, column=1, sticky="w", padx=(3, 8), pady=(4, 0)
        )

        reference = ttk.Frame(geometry_module)
        reference.pack(fill="x", pady=(4, 0))
        ttk.Label(reference, text="边码/标准引用").grid(row=0, column=0, sticky="w")
        self.reference_kind = tk.StringVar(value="")
        ttk.Combobox(
            reference,
            textvariable=self.reference_kind,
            values=("", *gold.REFERENCE_KINDS),
            state="readonly",
            width=18,
        ).grid(row=0, column=1, padx=3)
        self.reference_system = tk.StringVar()
        ttk.Entry(reference, textvariable=self.reference_system, width=10).grid(
            row=0, column=2, padx=3
        )
        self.reference_label = tk.StringVar()
        ttk.Entry(reference, textvariable=self.reference_label, width=8).grid(
            row=0, column=3, padx=3
        )
        self.reference_anchor = tk.StringVar()
        ttk.Entry(reference, textvariable=self.reference_anchor, width=14).grid(
            row=0, column=4, padx=3
        )

        character_module = ttk.LabelFrame(
            character_tab,
            text="2. 字符模块 · 原样转录与标签",
            padding=6,
        )
        character_module.pack(fill="x")
        self.text = tk.Text(character_module, height=3, wrap="word", undo=True)
        self.text.pack(fill="x")

        labels = ttk.Frame(character_module)
        labels.pack(fill="x", pady=6)
        ttk.Label(labels, text="语言").grid(row=0, column=0, sticky="w")
        self.language = tk.StringVar(value=self.default_language)
        ttk.Combobox(
            labels,
            textvariable=self.language,
            values=gold.LANGUAGES,
            state="readonly",
            width=10,
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
        ttk.Label(labels, text="note_id").grid(row=1, column=0, sticky="w", pady=(4, 0))
        self.note_id = tk.StringVar()
        ttk.Entry(labels, textvariable=self.note_id, width=20).grid(
            row=1, column=1, columnspan=2, sticky="ew", padx=(4, 12), pady=(4, 0)
        )

        spans = ttk.LabelFrame(character_module, text="行内排印（先在转录框选字）", padding=4)
        spans.pack(fill="x", pady=(0, 5))
        span_controls = ttk.Frame(spans)
        span_controls.pack(fill="x")
        self.span_style = tk.StringVar(value="none")
        ttk.Combobox(
            span_controls,
            textvariable=self.span_style,
            values=("none", *gold.INLINE_STYLES),
            state="readonly",
            width=12,
        ).pack(side="left")
        self.span_role = tk.StringVar(value="none")
        ttk.Combobox(
            span_controls,
            textvariable=self.span_role,
            values=gold.INLINE_SEMANTIC_ROLES,
            state="readonly",
            width=17,
        ).pack(side="left", padx=4)
        self.span_target = tk.StringVar()
        ttk.Entry(span_controls, textvariable=self.span_target, width=14).pack(
            side="left", padx=(0, 4)
        )
        ttk.Button(span_controls, text="+选区", command=self.add_inline_span).pack(
            side="left"
        )
        ttk.Button(span_controls, text="删span", command=self.delete_inline_span).pack(
            side="right"
        )
        self.span_list = tk.Listbox(spans, height=3, exportselection=False)
        self.span_list.pack(fill="x", pady=(4, 0))
        ttk.Button(
            character_module,
            text="✓ 核验字符",
            command=self.verify_transcription,
        ).pack(anchor="w")

        coverage = ttk.LabelFrame(page_tab, text="页面覆盖门禁", padding=6)
        coverage.pack(fill="x", pady=(4, 0))
        if self.review_scope == "ocr-geometry":
            ttk.Label(coverage, text="Dev reference QA · 仅几何、文字、顺序与覆盖\nD 结构/排印语义延期；完成后仍为 draft。", wraplength=420).pack(anchor="w")
        self.exhaustive = tk.BooleanVar(value=False)
        ttk.Checkbutton(
            coverage,
            text="我已检查整页，所有可见文字行均已包含",
            variable=self.exhaustive,
        ).pack(anchor="w")
        self.structure_exhaustive = tk.BooleanVar(value=False)
        ttk.Checkbutton(
            coverage,
            text="结构角色、段落、叶面/栏位与边码已穷尽核验",
            variable=self.structure_exhaustive,
            state="disabled" if self.review_scope == "ocr-geometry" else "normal",
        ).pack(anchor="w")
        self.typography_exhaustive = tk.BooleanVar(value=False)
        ttk.Checkbutton(
            coverage,
            text="上标、下标、斜体、粗体、下划线与小型大写已穷尽核验",
            variable=self.typography_exhaustive,
            state="disabled" if self.review_scope == "ocr-geometry" else "normal",
        ).pack(anchor="w")
        reviewer_row = ttk.Frame(coverage)
        reviewer_row.pack(fill="x", pady=4)
        ttk.Label(reviewer_row, text="核验人").pack(side="left")
        self.reviewer = tk.StringVar()
        ttk.Entry(reviewer_row, textvariable=self.reviewer).pack(
            side="left", fill="x", expand=True, padx=5
        )
        ttk.Label(coverage, text="未解决问题（必须清空才可完成）").pack(anchor="w")
        self.notes = tk.Text(coverage, height=1, wrap="word")
        self.notes.pack(fill="x")

        actions = ttk.Frame(page_tab)
        actions.pack(fill="x", pady=8)
        ttk.Button(actions, text="保存草稿  ⌘S", command=self.save_draft).pack(
            side="left"
        )
        ttk.Button(actions, text=("完成 Dev reference QA" if self.review_scope == "ocr-geometry" else "完成并核验页面"), command=self.verify_page).pack(
            side="right"
        )

        root.bind("<Command-s>", lambda _event: self.save_draft())
        root.bind("<Control-s>", lambda _event: self.save_draft())
        root.bind("<Command-Return>", lambda _event: self.verify_next_stage())
        self.load_page(0)
        root.lift()
        root.attributes("-topmost", True)
        root.after(600, lambda: root.attributes("-topmost", False))

    def record_path(self, page_id: str | None = None) -> Path:
        return self.output_root / f"{page_id or self.page.name}.json"

    def load_page(self, position: int) -> None:
        from tkinter import messagebox

        page = self.pages[position]
        page_id = page.name
        self.page_position = position
        self.page = page
        path = self.record_path(page_id)
        self.disk_digest = workflow.digest(path)
        if path.exists():
            self.record = gold.load_record(path)
            gold.upgrade_record_to_v1_2(self.record)
        elif self.cgpg_mode:
            self.record = gold.draft_from_cgpg(page)
        else:
            self.record = gold.draft_from_image(page.image_path, page.name)
        try:
            gold.validate_record(
                self.record, require_complete=False, image_path=page.image_path
            )
        except gold.GoldError as error:
            messagebox.showerror("草稿损坏或图片不匹配", str(error))
            return
        if self.record["coverage"]["status"] == "verified":
            try:
                gold.validate_record(self.record, require_complete=True, image_path=page.image_path)
            except gold.GoldError as error:
                self.invalidate_page()
                messagebox.showwarning("旧完成状态无效，已恢复草稿", str(error) + "\n已保留所有逐行编辑；保存时仍会检查外部修改。")
        if self.review_scope == "ocr-geometry":
            self.invalidate_page()
        self.image = Image.open(page.image_path).convert("RGB")
        self.selected = 0 if self.record["lines"] else None
        coverage = self.record["coverage"]
        self.exhaustive.set(bool(coverage["all_visible_lines_exhaustively_reviewed"]))
        self.structure_exhaustive.set(
            bool(coverage.get("all_structure_exhaustively_reviewed", False))
        )
        self.typography_exhaustive.set(
            bool(coverage.get("all_typography_exhaustively_reviewed", False))
        )
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
        fit_scale = min(
            canvas_width / self.page.width, canvas_height / self.page.height
        )
        self.scale = fit_scale * self.zoom_factor
        size = (
            max(1, int(self.page.width * self.scale)),
            max(1, int(self.page.height * self.scale)),
        )
        display = self.image.resize(size, Image.Resampling.LANCZOS)
        self.display_photo = ImageTk.PhotoImage(display)
        self.canvas.delete("all")
        self.canvas.create_image(0, 0, image=self.display_photo, anchor="nw")
        self.canvas.configure(scrollregion=(0, 0, size[0], size[1]))
        self.canvas_zoom_label.configure(text=f"{round(self.zoom_factor * 100)}%")
        for index, line in enumerate(self.record["lines"]):
            left, top, right, bottom = [value * self.scale for value in line["bbox"]]
            geometry_verified = self.geometry_module_verified(line)
            transcription_verified = self.character_module_verified(line)
            if geometry_verified and transcription_verified:
                color = "#4ee06f"
            elif geometry_verified:
                color = "#4da3ff"
            elif transcription_verified:
                color = "#c77dff"
            else:
                color = "#ffad33"
            width = 3 if index == self.selected else 1
            if index == self.selected:
                color = "#ff3b5c"
            self.canvas.create_rectangle(
                left, top, right, bottom, outline=color, width=width
            )
        self.refresh_list()
        self.refresh_status()

    def set_zoom(self, factor: float) -> None:
        if self.page is None:
            return
        center_x = self.canvas.canvasx(self.canvas.winfo_width() / 2)
        center_y = self.canvas.canvasy(self.canvas.winfo_height() / 2)
        old_scale = self.scale
        self.zoom_factor = min(6.0, max(0.5, factor))
        self.render()
        if old_scale > 0:
            target_x = center_x * self.scale / old_scale
            target_y = center_y * self.scale / old_scale
            region = self.canvas.cget("scrollregion").split()
            region_width = float(region[2]) if len(region) == 4 else 1.0
            region_height = float(region[3]) if len(region) == 4 else 1.0
            self.canvas.xview_moveto(
                max(0.0, (target_x - self.canvas.winfo_width() / 2) / region_width)
            )
            self.canvas.yview_moveto(
                max(0.0, (target_y - self.canvas.winfo_height() / 2) / region_height)
            )

    def zoom_in(self) -> None:
        self.set_zoom(self.zoom_factor * 1.25)

    def zoom_out(self) -> None:
        self.set_zoom(self.zoom_factor / 1.25)

    def reset_zoom(self) -> None:
        self.zoom_factor = 1.0
        self.render()
        self.canvas.xview_moveto(0)
        self.canvas.yview_moveto(0)

    def zoom_with_wheel(self, event):
        self.set_zoom(self.zoom_factor * (1.15 if event.delta > 0 else 1 / 1.15))
        return "break"

    def refresh_list(self) -> None:
        self.listbox.delete(0, "end")
        for index, line in enumerate(self.record["lines"]):
            geometry = "G" if self.geometry_module_verified(line) else "·"
            transcription = "T" if self.character_module_verified(line) else "·"
            text = line["text"].replace("\n", " ")
            self.listbox.insert(
                "end",
                f"[{geometry}{transcription}] {index + 1:03d}  {text[:48]}",
            )
        if self.selected is not None and self.selected < self.listbox.size():
            self.listbox.selection_set(self.selected)
            self.listbox.see(self.selected)

    def refresh_status(self) -> None:
        geometry_verified = sum(
            self.geometry_module_verified(line) for line in self.record["lines"]
        )
        transcription_verified = sum(
            self.character_module_verified(line) for line in self.record["lines"]
        )
        self.status_label.configure(
            text=(
                f"几何 {geometry_verified}/{len(self.record['lines'])} · "
                f"字符 {transcription_verified}/{len(self.record['lines'])} · "
                f"页面 {self.page_review_status()}"
            )
        )

    def page_review_status(self):
        if self.review_scope == "ocr-geometry":
            valid = workflow.receipt_valid(self.record_path(), self.record, self.page.image_path)
            return "Dev reference QA 已完成（非 Gold）" if valid else "Dev reference QA 待完成（非 Gold）"
        return self.record["coverage"]["status"]

    def show_validation_error(self, title, error):
        from tkinter import messagebox
        index = workflow.error_line(self.record, str(error))
        if index is not None:
            self.selected = index
            self.render()
            self.load_form()
        location = f"\n已定位第 {index + 1} 行。" if index is not None else ""
        messagebox.showerror(title, str(error) + location + "\n已编辑内容保留在窗口中；可保存草稿后继续修订。")

    def geometry_module_verified(self, line: dict) -> bool:
        if self.review_scope == "ocr-geometry":
            return line["geometry_status"] == "human_verified"
        return line["geometry_status"] == "human_verified" and line.get(
            "structure_status", "human_verified"
        ) == "human_verified"

    def character_module_verified(self, line: dict) -> bool:
        if self.review_scope == "ocr-geometry":
            return line["transcription_status"] == "human_verified"
        return line["transcription_status"] == "human_verified" and line.get(
            "typography_status", "human_verified"
        ) == "human_verified"

    def select_from_list(self, _event=None) -> None:
        selection = self.listbox.curselection()
        if selection:
            self.select_line(selection[0])

    def select_at(self, event) -> None:
        if event.state & 0x0001:
            return
        x = self.canvas.canvasx(event.x) / self.scale
        y = self.canvas.canvasy(event.y) / self.scale
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
        geometry = "已核验" if self.geometry_module_verified(line) else "待核验"
        transcription = "已核验" if self.character_module_verified(line) else "待核验"
        self.line_label.configure(
            text=(
                f"行 {self.selected + 1} · {line['line_id']} · "
                f"几何{geometry} · 字符{transcription}"
            )
        )
        self.text.delete("1.0", "end")
        self.text.insert("1.0", line["text"])
        self.language.set(line["language"])
        self.content_class.set(line["content_class"])
        self.paragraph_role.set(line.get("paragraph_role", "not_applicable"))
        self.leaf_id.set(line.get("leaf_id") or "")
        self.column_id.set(line.get("column_id") or "")
        self.hierarchy_level.set(
            "" if line.get("hierarchy_level") is None else str(line["hierarchy_level"])
        )
        self.note_id.set(line.get("note_id") or "")
        reference = line.get("canonical_reference") or {}
        self.reference_kind.set(reference.get("kind", ""))
        self.reference_system.set(reference.get("system", ""))
        self.reference_label.set(reference.get("label", ""))
        self.reference_anchor.set(reference.get("anchor_line_id") or "")
        self.refresh_span_list()
        for variable, value in zip(self.box_vars, line["bbox"]):
            variable.set(str(int(value) if float(value).is_integer() else value))
        self.render_zoom(line["bbox"])
        self.loading_form = False

    def refresh_span_list(self) -> None:
        self.span_list.delete(0, "end")
        if self.selected is None or self.selected >= len(self.record["lines"]):
            return
        for span in self.record["lines"][self.selected].get("inline_spans", []):
            styles = ",".join(span["styles"]) or "—"
            role = span["semantic_role"]
            target = f" →{span['target_id']}" if span["target_id"] else ""
            self.span_list.insert(
                "end",
                f"{span['start_char']}:{span['end_char']} [{styles}; {role}]{target} {span['text']}",
            )

    def render_zoom(self, bbox) -> None:
        left, top, right, bottom = map(int, bbox)
        padding = max(8, int((bottom - top) * 0.8))
        crop = self.image.crop(
            (
                max(0, left - padding),
                max(0, top - padding),
                min(self.page.width, right + padding),
                min(self.page.height, bottom + padding),
            )
        )
        width = max(self.zoom.winfo_width(), 480)
        height = max(self.zoom.winfo_height(), 115)
        factor = min(width / crop.width, height / crop.height)
        crop = crop.resize(
            (max(1, int(crop.width * factor)), max(1, int(crop.height * factor))),
            Image.Resampling.LANCZOS,
        )
        self.zoom_photo = ImageTk.PhotoImage(crop)
        self.zoom.delete("all")
        self.zoom.create_image(
            width // 2, height // 2, image=self.zoom_photo, anchor="center"
        )

    def invalidate_page(self) -> None:
        coverage = self.record["coverage"]
        coverage["status"] = "draft"
        coverage["verified_at"] = None

    def commit_form(self) -> None:
        if (
            self.loading_form
            or self.selected is None
            or self.selected >= len(self.record["lines"])
        ):
            return
        line = self.record["lines"][self.selected]
        proposed_text = unicodedata.normalize("NFC", self.text.get("1.0", "end-1c"))
        proposed = {
            "text": proposed_text,
            "language": self.language.get(),
            "content_class": self.content_class.get(),
        }
        if any(line[key] != value for key, value in proposed.items()):
            if line["text"] != proposed_text and line.get("inline_spans"):
                line["inline_spans"] = []
            line.update(proposed)
            gold.invalidate_line_transcription(line)
            self.invalidate_page()
        if "structure_status" in line:
            reference_values = (
                self.reference_kind.get().strip(),
                self.reference_system.get().strip(),
                self.reference_label.get().strip(),
                self.reference_anchor.get().strip(),
            )
            reference = (
                {
                    "kind": reference_values[0],
                    "system": reference_values[1],
                    "label": reference_values[2],
                    "anchor_line_id": reference_values[3] or None,
                }
                if any(reference_values)
                else None
            )
            structure_proposed = {
                "paragraph_role": self.paragraph_role.get(),
                "leaf_id": self.leaf_id.get().strip() or None,
                "column_id": self.column_id.get().strip() or None,
                "hierarchy_level": self._parsed_hierarchy_level(),
                "canonical_reference": reference,
            }
            if any(line[key] != value for key, value in structure_proposed.items()):
                line.update(structure_proposed)
                gold.invalidate_line_structure(line)
                self.invalidate_page()
            proposed_note_id = self.note_id.get().strip() or None
            if line["note_id"] != proposed_note_id:
                line["note_id"] = proposed_note_id
                gold.invalidate_line_typography(line)
                self.invalidate_page()

    def _parsed_hierarchy_level(self):
        from tkinter import messagebox

        value = self.hierarchy_level.get().strip()
        if not value:
            return None
        try:
            level = int(value)
        except ValueError:
            messagebox.showerror("目录层级无效", "目录层级必须是 1–32 的整数。")
            return None
        if not 1 <= level <= 32:
            messagebox.showerror("目录层级无效", "目录层级必须是 1–32 的整数。")
            return None
        return level

    def add_inline_span(self) -> None:
        from tkinter import messagebox

        if self.selected is None:
            return
        try:
            start_index = self.text.index("sel.first")
            end_index = self.text.index("sel.last")
        except self.tk.TclError:
            messagebox.showerror("没有选区", "请先在转录框中选中需要标注的字符。")
            return
        start = int(self.text.count("1.0", start_index, "chars")[0])
        end = int(self.text.count("1.0", end_index, "chars")[0])
        style = self.span_style.get()
        role = self.span_role.get()
        target = self.span_target.get().strip() or None
        if style == "none" and role == "none":
            messagebox.showerror("缺少标注", "请选择一种排印样式或语义角色。")
            return
        if role == "footnote_marker" and not target:
            messagebox.showerror("缺少目标", "脚注标记必须填写目标 note_id。")
            return
        self.commit_form()
        line = self.record["lines"][self.selected]
        spans = line["inline_spans"]
        exact = next(
            (span for span in spans if span["start_char"] == start and span["end_char"] == end),
            None,
        )
        if exact is not None:
            if style != "none" and style not in exact["styles"]:
                exact["styles"].append(style)
            if role != "none":
                exact["semantic_role"] = role
            if target:
                exact["target_id"] = target
        else:
            if any(start < span["end_char"] and end > span["start_char"] for span in spans):
                messagebox.showerror("选区重叠", "span 不能部分重叠；可对完全相同选区追加样式。")
                return
            used = {span["span_id"] for span in spans}
            counter = len(used)
            while f"{line['line_id']}-span-{counter:04d}" in used:
                counter += 1
            spans.append(
                {
                    "span_id": f"{line['line_id']}-span-{counter:04d}",
                    "start_char": start,
                    "end_char": end,
                    "text": line["text"][start:end],
                    "styles": [] if style == "none" else [style],
                    "semantic_role": role,
                    "target_id": target,
                    "bbox": None,
                }
            )
            spans.sort(key=lambda span: (span["start_char"], span["end_char"], span["span_id"]))
        gold.invalidate_line_typography(line)
        self.invalidate_page()
        self.refresh_span_list()
        self.render()

    def delete_inline_span(self) -> None:
        if self.selected is None:
            return
        selection = self.span_list.curselection()
        if not selection:
            return
        line = self.record["lines"][self.selected]
        del line["inline_spans"][selection[0]]
        gold.invalidate_line_typography(line)
        self.invalidate_page()
        self.refresh_span_list()
        self.render()

    def apply_box(self) -> None:
        from tkinter import messagebox

        if self.selected is None:
            return
        try:
            bbox = [float(variable.get()) for variable in self.box_vars]
            left, top, right, bottom = bbox
            if not (
                0 <= left < right <= self.page.width
                and 0 <= top < bottom <= self.page.height
            ):
                raise ValueError
        except ValueError:
            messagebox.showerror(
                "框无效", "坐标必须位于页面内，且 left < right、top < bottom。"
            )
            return
        line = self.record["lines"][self.selected]
        if line["bbox"] != bbox:
            line["bbox"] = bbox
            gold.invalidate_line_geometry(line)
            self.invalidate_page()
        self.render()
        self.load_form()

    def advance_line(self) -> None:
        if self.selected is not None and self.selected + 1 < len(self.record["lines"]):
            self.select_line(self.selected + 1)
        else:
            self.load_form()

    def verify_geometry(self) -> None:
        from tkinter import messagebox

        if self.selected is None:
            return
        self.commit_form()
        try:
            self.record = workflow.mark_line(self.record, self.selected, "geometry", self.review_scope, self.page.image_path)
        except gold.GoldError as error:
            self.show_validation_error("不能核验几何/结构", error)
            return
        self.invalidate_page()
        self.render()
        self.advance_line()

    def verify_transcription(self) -> None:
        from tkinter import messagebox

        if self.selected is None:
            return
        self.commit_form()
        line = self.record["lines"][self.selected]
        try:
            self.record = workflow.mark_line(self.record, self.selected, "transcription", self.review_scope, self.page.image_path)
        except gold.GoldError as error:
            self.show_validation_error("不能核验字符", error)
            return
        self.invalidate_page()
        self.render()
        self.advance_line()

    def verify_next_stage(self) -> None:
        if self.selected is None:
            return
        line = self.record["lines"][self.selected]
        if not self.geometry_module_verified(line):
            self.verify_geometry()
        else:
            self.verify_transcription()

    def start_new_box(self, event) -> None:
        x, y = self.canvas.canvasx(event.x), self.canvas.canvasy(event.y)
        self.drag_start = (x, y)
        self.drag_rectangle = self.canvas.create_rectangle(
            x, y, x, y, outline="#4da3ff", width=2
        )

    def drag_new_box(self, event) -> None:
        if self.drag_start and self.drag_rectangle:
            x, y = self.canvas.canvasx(event.x), self.canvas.canvasy(event.y)
            self.canvas.coords(
                self.drag_rectangle,
                self.drag_start[0],
                self.drag_start[1],
                x,
                y,
            )

    def finish_new_box(self, event) -> None:
        if not self.drag_start:
            return
        x1, y1 = self.drag_start
        event_x = self.canvas.canvasx(event.x)
        event_y = self.canvas.canvasy(event.y)
        self.drag_start = None
        if abs(event_x - x1) < 4 or abs(event_y - y1) < 4:
            self.render()
            return
        self.commit_form()
        left, right = sorted((x1 / self.scale, event_x / self.scale))
        top, bottom = sorted((y1 / self.scale, event_y / self.scale))
        left, top = max(0, left), max(0, top)
        right, bottom = min(self.page.width, right), min(self.page.height, bottom)
        used = {line["line_id"] for line in self.record["lines"]}
        counter = len(used)
        while f"{self.page.name}-human-{counter:04d}" in used:
            counter += 1
        new_line = {
                "line_id": f"{self.page.name}-human-{counter:04d}",
                "reading_order": len(self.record["lines"]),
                "bbox": [
                    round(left, 2),
                    round(top, 2),
                    round(right, 2),
                    round(bottom, 2),
                ],
                "text": "",
                "language": self.default_language,
                "content_class": "unclassified",
                "geometry_status": "candidate_unverified",
                "transcription_status": "candidate_unverified",
                "source": "human_draft",
            }
        if self.record.get("schema_version") == gold.SCHEMA_VERSION:
            new_line.update(
                structure_status="candidate_unverified",
                typography_status="candidate_unverified",
                paragraph_role="unclassified",
                leaf_id=None,
                column_id=None,
                hierarchy_level=None,
                note_id=None,
                canonical_reference=None,
                inline_spans=[],
            )
        self.record["lines"].append(new_line)
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
        self.record["lines"][self.selected], self.record["lines"][target] = (
            self.record["lines"][target],
            self.record["lines"][self.selected],
        )
        gold.invalidate_line_geometry(self.record["lines"][self.selected])
        gold.invalidate_line_geometry(self.record["lines"][target])
        self.selected = target
        self.renumber()
        self.invalidate_page()
        self.render()
        self.load_form()

    def delete_line(self) -> None:
        from tkinter import messagebox

        if self.selected is None or not messagebox.askyesno(
            "删除行", "确定删除当前行框？草稿保存前仍可关闭窗口放弃。"
        ):
            return
        deleted = self.selected
        del self.record["lines"][deleted]
        self.renumber()
        for line in self.record["lines"][deleted:]:
            gold.invalidate_line_geometry(line)
        self.invalidate_page()
        self.selected = (
            min(self.selected, len(self.record["lines"]) - 1)
            if self.record["lines"]
            else None
        )
        self.render()
        self.load_form()

    def update_coverage_form(self) -> None:
        coverage = self.record["coverage"]
        before = dict(coverage)
        coverage["all_visible_lines_exhaustively_reviewed"] = bool(
            self.exhaustive.get()
        )
        if self.record.get("schema_version") == gold.SCHEMA_VERSION:
            coverage["all_structure_exhaustively_reviewed"] = bool(
                self.structure_exhaustive.get()
            )
            coverage["all_typography_exhaustively_reviewed"] = bool(
                self.typography_exhaustive.get()
            )
        coverage["reviewer"] = self.reviewer.get().strip()
        coverage["unresolved_notes"] = self.notes.get("1.0", "end-1c")
        if coverage != before:
            self.invalidate_page()

    def save_draft(self, quiet: bool = False) -> bool:
        from tkinter import messagebox

        self.commit_form()
        self.update_coverage_form()
        try:
            gold.validate_record(
                self.record, require_complete=False, image_path=self.page.image_path
            )
            self.disk_digest = workflow.save(self.record_path(), self.record, self.disk_digest, self.page.image_path)
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
            candidate = workflow.complete(self.record, self.reviewer.get(), self.review_scope, self.page.image_path)
            self.disk_digest = workflow.save(self.record_path(), candidate, self.disk_digest, self.page.image_path, dev_complete=self.review_scope == "ocr-geometry")
            self.record = candidate
        except (gold.GoldError, OSError) as error:
            self.show_validation_error("页面尚不能完成", error)
            return
        self.render()
        messagebox.showinfo(
            "页面已核验",
            ("Dev reference QA 已完成：几何、文字、阅读顺序和可见行覆盖已核验。记录仍为 draft；独立凭据不代表完整 Gold，D 语义核验延期。" if self.review_scope == "ocr-geometry" else "此页的几何与字符模块均已核验，并通过 closed-world coverage contract。"),
        )

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
    parser.add_argument(
        "--corpus-root", type=Path, help="legacy CGPG PAGE-XML directory"
    )
    parser.add_argument(
        "--images", nargs="+", type=Path, help="PNG/JPEG/TIFF page images"
    )
    parser.add_argument("--output-root", type=Path)
    parser.add_argument("--review-scope", choices=("full-gold", "ocr-geometry"), default="full-gold")
    parser.add_argument(
        "--all-holdout",
        action="store_true",
        help="annotate all 12 frozen holdout pages",
    )
    parser.add_argument("--pages", nargs="+", choices=gold.HOLDOUT_PAGE_IDS)
    return parser.parse_args()


def main() -> int:
    import tkinter as tk
    from tkinter import filedialog

    args = parse_args()
    if args.corpus_root and args.images:
        raise SystemExit("choose either --corpus-root or --images, not both")
    if not args.corpus_root and (args.pages or args.all_holdout):
        raise SystemExit("--pages/--all-holdout require --corpus-root")
    root = tk.Tk()
    root.withdraw()
    if args.corpus_root:
        page_ids = list(
            args.pages
            or (gold.HOLDOUT_PAGE_IDS if args.all_holdout else gold.FORENSIC_PAGE_IDS)
        )
        missing = [
            page_id
            for page_id in page_ids
            if not (args.corpus_root / f"{page_id}.xml").is_file()
        ]
        if missing:
            raise SystemExit(f"missing corpus pages: {', '.join(missing)}")
        pages = [
            cgpg.load_page(args.corpus_root / f"{page_id}.xml") for page_id in page_ids
        ]
        cgpg_mode = True
        output_root = args.output_root or CGPG_OUTPUT
    else:
        image_paths = list(args.images or ())
        if not image_paths:
            selected = filedialog.askopenfilenames(
                title="选择要制作 closed-world gold 的页面图像",
                filetypes=[
                    ("Page images", "*.png *.jpg *.jpeg *.tif *.tiff"),
                    ("All files", "*"),
                ],
            )
            image_paths = [Path(path) for path in selected]
        if not image_paths:
            root.destroy()
            return 0
        missing = [str(path) for path in image_paths if not path.is_file()]
        if missing:
            raise SystemExit(f"missing page images: {', '.join(missing)}")
        pages = [load_image_page(path) for path in image_paths]
        names = [page.name for page in pages]
        if len(set(names)) != len(names):
            raise SystemExit("selected image filenames must have unique stems")
        cgpg_mode = False
        output_root = args.output_root or DEFAULT_OUTPUT
    root.deiconify()
    GoldAnnotator(root, pages, output_root.resolve(), cgpg_mode=cgpg_mode, review_scope=args.review_scope)
    root.mainloop()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
