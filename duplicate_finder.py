#!/usr/bin/env python3
"""
Duplicate Image Finder & Cleanup Tool
Cross-platform (macOS, Linux, Windows) GUI application for finding
and managing duplicate images using MD5 hashing.
"""

import hashlib
import os
import platform
import subprocess
import sys
import threading
import tkinter as tk
import warnings
from collections import defaultdict
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime
from pathlib import Path
from tkinter import filedialog, messagebox, ttk

try:
    from PIL import Image, ImageTk
    from send2trash import send2trash
    warnings.filterwarnings("ignore", category=UserWarning, module="PIL")
except ImportError:
    print("Missing dependencies. Install with:\n  pip install -r requirements.txt")
    sys.exit(1)

IMAGE_EXTENSIONS = {
    ".jpg", ".jpeg", ".png", ".gif", ".bmp", ".tiff", ".tif",
    ".webp", ".heic", ".heif", ".avif", ".ico", ".svg",
}
THUMB_SIZE = (250, 250)
SCAN_CHUNK = 500  # UI update interval during scan


# ──────────────────────────────────────────────
# Scanning engine
# ──────────────────────────────────────────────

def collect_image_paths(root_dir, progress_cb=None):
    paths = []
    for dirpath, _, filenames in os.walk(root_dir):
        for f in filenames:
            if Path(f).suffix.lower() in IMAGE_EXTENSIONS:
                paths.append(os.path.join(dirpath, f))
                if progress_cb and len(paths) % SCAN_CHUNK == 0:
                    progress_cb(f"Collecting files… {len(paths)}")
    return paths


def file_md5(path, chunk_size=1 << 16):
    h = hashlib.md5()
    try:
        with open(path, "rb") as f:
            while True:
                chunk = f.read(chunk_size)
                if not chunk:
                    break
                h.update(chunk)
    except OSError:
        return None
    return h.hexdigest()


def find_duplicates(paths, progress_cb=None, cancel_event=None):
    """
    Find exact duplicate files via MD5 hashing.
    Returns list of groups, each group is a list of file paths.
    """
    # Pass 1: exact byte-identical files (fast, via MD5)
    md5_map = defaultdict(list)
    total = len(paths)

    def update(i, label="Hashing"):
        if progress_cb and i % 50 == 0:
            progress_cb(f"{label}… {i}/{total}", i / total)

    with ThreadPoolExecutor(max_workers=os.cpu_count() or 4) as pool:
        futures = {pool.submit(file_md5, p): p for p in paths}
        for i, fut in enumerate(as_completed(futures)):
            if cancel_event and cancel_event.is_set():
                pool.shutdown(wait=False, cancel_futures=True)
                return []
            path = futures[fut]
            md5 = fut.result()
            if md5:
                md5_map[md5].append(path)
            update(i, "MD5 hashing")

    exact_groups = [g for g in md5_map.values() if len(g) > 1]

    if progress_cb:
        progress_cb("Done", 1.0)

    return exact_groups


def file_info(path):
    try:
        stat = os.stat(path)
        size = stat.st_size
        mtime = datetime.fromtimestamp(stat.st_mtime).strftime("%Y-%m-%d %H:%M")
    except OSError:
        size, mtime = 0, "?"
    if size < 1024:
        sz = f"{size} B"
    elif size < 1024 * 1024:
        sz = f"{size / 1024:.1f} KB"
    else:
        sz = f"{size / (1024 * 1024):.1f} MB"
    return sz, mtime


def image_dimensions(path):
    try:
        with Image.open(path) as img:
            return f"{img.width}x{img.height}"
    except Exception:
        return "?"


# ──────────────────────────────────────────────
# GUI
# ──────────────────────────────────────────────

class App(tk.Tk):
    def __init__(self):
        super().__init__()
        self.title("Duplicate Image Finder")
        self.geometry("1100x720")
        self.minsize(900, 550)
        self._configure_style()

        self.groups = []         # list[list[str]]
        self.current_group = 0
        self.thumb_cache = {}
        self.cancel_event = threading.Event()
        self.check_vars = []     # BooleanVar per image in current group

        self._build_toolbar()
        self._build_main()
        self._build_statusbar()
        self._show_welcome()

    # ── style ──
    def _configure_style(self):
        style = ttk.Style(self)
        try:
            if platform.system() == "Darwin":
                style.theme_use("aqua")
            elif platform.system() == "Windows":
                style.theme_use("vista")
            else:
                style.theme_use("clam")
        except tk.TclError:
            style.theme_use("default")

    # ── toolbar ──
    def _build_toolbar(self):
        tb = ttk.Frame(self, padding=6)
        tb.pack(fill=tk.X)

        ttk.Button(tb, text="Choose Folder…", command=self._on_browse).pack(side=tk.LEFT)
        self.path_var = tk.StringVar(value="")
        ttk.Entry(tb, textvariable=self.path_var, width=50).pack(side=tk.LEFT, padx=(6, 0), fill=tk.X, expand=True)
        self.scan_btn = ttk.Button(tb, text="Scan", command=self._on_scan)
        self.scan_btn.pack(side=tk.LEFT, padx=(6, 0))
        self.cancel_btn = ttk.Button(tb, text="Cancel", command=self._on_cancel, state=tk.DISABLED)
        self.cancel_btn.pack(side=tk.LEFT, padx=(6, 0))

    # ── main area ──
    def _build_main(self):
        self.main = ttk.Frame(self)
        self.main.pack(fill=tk.BOTH, expand=True, padx=8, pady=(0, 4))

        # Left: group list
        left = ttk.LabelFrame(self.main, text="Duplicate Groups", padding=4)
        left.pack(side=tk.LEFT, fill=tk.Y, padx=(0, 4))

        self.group_listbox = tk.Listbox(left, width=28, font=("TkDefaultFont", 11))
        self.group_listbox.pack(fill=tk.BOTH, expand=True)
        self.group_listbox.bind("<<ListboxSelect>>", self._on_group_select)

        nav = ttk.Frame(left)
        nav.pack(fill=tk.X, pady=(4, 0))
        ttk.Button(nav, text="< Prev", command=self._prev_group).pack(side=tk.LEFT, expand=True, fill=tk.X)
        ttk.Button(nav, text="Next >", command=self._next_group).pack(side=tk.LEFT, expand=True, fill=tk.X)

        # Right: image detail panel
        self.detail = ttk.Frame(self.main)
        self.detail.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)

        # Scrollable canvas for images
        self.canvas = tk.Canvas(self.detail, highlightthickness=0)
        self.vscroll = ttk.Scrollbar(self.detail, orient=tk.VERTICAL, command=self.canvas.yview)
        self.canvas.configure(yscrollcommand=self.vscroll.set)
        self.vscroll.pack(side=tk.RIGHT, fill=tk.Y)
        self.canvas.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)

        self.inner = ttk.Frame(self.canvas)
        self.canvas_window = self.canvas.create_window((0, 0), window=self.inner, anchor=tk.NW)
        self.inner.bind("<Configure>", lambda e: self.canvas.configure(scrollregion=self.canvas.bbox("all")))
        self.canvas.bind("<Configure>", self._on_canvas_resize)

        # Bind mouse wheel
        self.canvas.bind_all("<MouseWheel>", self._on_mousewheel)          # Windows/macOS
        self.canvas.bind_all("<Button-4>", self._on_mousewheel_linux)
        self.canvas.bind_all("<Button-5>", self._on_mousewheel_linux)

        # Action bar at bottom of detail
        self.action_bar = ttk.Frame(self.main)
        self.action_bar.pack(side=tk.BOTTOM, fill=tk.X, pady=(4, 0))

    def _on_canvas_resize(self, event):
        self.canvas.itemconfig(self.canvas_window, width=event.width)

    def _on_mousewheel(self, event):
        self.canvas.yview_scroll(-1 * (event.delta // 120 or (1 if event.delta > 0 else -1)), "units")

    def _on_mousewheel_linux(self, event):
        self.canvas.yview_scroll(-1 if event.num == 4 else 1, "units")

    # ── status bar ──
    def _build_statusbar(self):
        sb = ttk.Frame(self, padding=(8, 2))
        sb.pack(fill=tk.X, side=tk.BOTTOM)

        self.status_var = tk.StringVar(value="Ready")
        ttk.Label(sb, textvariable=self.status_var).pack(side=tk.LEFT)
        self.progress = ttk.Progressbar(sb, length=200, mode="determinate")
        self.progress.pack(side=tk.RIGHT)

    # ── welcome screen ──
    def _show_welcome(self):
        for w in self.inner.winfo_children():
            w.destroy()
        ttk.Label(
            self.inner,
            text="Choose a folder and click Scan to find duplicate images.",
            font=("TkDefaultFont", 14),
            padding=40,
        ).pack(expand=True)

    # ── toolbar actions ──
    def _on_browse(self):
        d = filedialog.askdirectory(title="Select folder to scan")
        if d:
            self.path_var.set(d)

    def _on_scan(self):
        folder = self.path_var.get().strip()
        if not folder or not os.path.isdir(folder):
            messagebox.showwarning("Invalid folder", "Please choose a valid folder first.")
            return

        self.groups.clear()
        self.group_listbox.delete(0, tk.END)
        self._show_welcome()
        self.cancel_event.clear()
        self.scan_btn.configure(state=tk.DISABLED)
        self.cancel_btn.configure(state=tk.NORMAL)

        def progress_cb(msg, frac=None):
            self.after(0, lambda: self.status_var.set(msg))
            if frac is not None:
                self.after(0, lambda: self.progress.configure(value=frac * 100))

        def run():
            progress_cb("Collecting image files…", 0.0)
            paths = collect_image_paths(folder, progress_cb)
            progress_cb(f"Found {len(paths)} images. Scanning for duplicates…", 0.05)
            groups = find_duplicates(paths, progress_cb, self.cancel_event)
            # Sort: larger groups first, then by first file name
            groups.sort(key=lambda g: (-len(g), g[0]))
            self.after(0, lambda: self._scan_done(groups))

        threading.Thread(target=run, daemon=True).start()

    def _on_cancel(self):
        self.cancel_event.set()
        self.status_var.set("Cancelling…")

    def _scan_done(self, groups):
        self.scan_btn.configure(state=tk.NORMAL)
        self.cancel_btn.configure(state=tk.DISABLED)

        if self.cancel_event.is_set():
            self.status_var.set("Scan cancelled.")
            self.progress.configure(value=0)
            return

        self.groups = groups
        self.group_listbox.delete(0, tk.END)
        for i, g in enumerate(groups):
            self.group_listbox.insert(tk.END, f"Group {i + 1} ({len(g)} files)")

        total_dupes = sum(len(g) - 1 for g in groups)
        self.status_var.set(f"Found {len(groups)} groups with {total_dupes} duplicates.")
        self.progress.configure(value=100)

        if groups:
            self.group_listbox.selection_set(0)
            self._show_group(0)
        else:
            self._show_welcome()
            for w in self.inner.winfo_children():
                w.destroy()
            ttk.Label(self.inner, text="No duplicates found!", font=("TkDefaultFont", 14), padding=40).pack()

    # ── group navigation ──
    def _on_group_select(self, event=None):
        sel = self.group_listbox.curselection()
        if sel:
            self._show_group(sel[0])

    def _prev_group(self):
        if not self.groups:
            return
        idx = max(0, self.current_group - 1)
        self.group_listbox.selection_clear(0, tk.END)
        self.group_listbox.selection_set(idx)
        self.group_listbox.see(idx)
        self._show_group(idx)

    def _next_group(self):
        if not self.groups:
            return
        idx = min(len(self.groups) - 1, self.current_group + 1)
        self.group_listbox.selection_clear(0, tk.END)
        self.group_listbox.selection_set(idx)
        self.group_listbox.see(idx)
        self._show_group(idx)

    # ── display a duplicate group ──
    def _show_group(self, idx):
        self.current_group = idx
        group = self.groups[idx]

        for w in self.inner.winfo_children():
            w.destroy()
        for w in self.action_bar.winfo_children():
            w.destroy()

        self.check_vars = []

        header = ttk.Frame(self.inner)
        header.pack(fill=tk.X, padx=8, pady=(8, 4))
        ttk.Label(header, text=f"Group {idx + 1} of {len(self.groups)}", font=("TkDefaultFont", 13, "bold")).pack(side=tk.LEFT)

        for i, path in enumerate(group):
            self._add_image_row(path, i)

        # Action bar
        ttk.Button(self.action_bar, text="Select All Except First", command=self._select_all_except_first).pack(side=tk.LEFT, padx=4)
        ttk.Button(self.action_bar, text="Select All Except Largest", command=self._select_all_except_largest).pack(side=tk.LEFT, padx=4)
        ttk.Button(self.action_bar, text="Deselect All", command=self._deselect_all).pack(side=tk.LEFT, padx=4)
        ttk.Separator(self.action_bar, orient=tk.VERTICAL).pack(side=tk.LEFT, fill=tk.Y, padx=8)
        ttk.Button(self.action_bar, text="Move Selected to Trash", command=self._trash_selected).pack(side=tk.LEFT, padx=4)
        ttk.Button(self.action_bar, text="Delete Selected Permanently", command=self._delete_selected).pack(side=tk.LEFT, padx=4)

        self.canvas.yview_moveto(0)

    def _add_image_row(self, path, index):
        row = ttk.Frame(self.inner, padding=6)
        row.pack(fill=tk.X, padx=8, pady=2)

        var = tk.BooleanVar(value=False)
        self.check_vars.append(var)
        cb = ttk.Checkbutton(row, variable=var)
        cb.pack(side=tk.LEFT, padx=(0, 8))

        # Thumbnail
        thumb_label = ttk.Label(row)
        thumb_label.pack(side=tk.LEFT, padx=(0, 10))
        self._load_thumb_async(path, thumb_label)

        # File info
        info_frame = ttk.Frame(row)
        info_frame.pack(side=tk.LEFT, fill=tk.X, expand=True)

        name = os.path.basename(path)
        ttk.Label(info_frame, text=name, font=("TkDefaultFont", 11, "bold")).pack(anchor=tk.W)
        ttk.Label(info_frame, text=path, foreground="gray").pack(anchor=tk.W)

        sz, mtime = file_info(path)
        dims = image_dimensions(path)
        ttk.Label(info_frame, text=f"{dims}  |  {sz}  |  Modified: {mtime}").pack(anchor=tk.W)

        # Open button
        ttk.Button(row, text="Open", command=lambda p=path: self._open_file(p)).pack(side=tk.RIGHT, padx=4)
        ttk.Button(row, text="Show in Folder", command=lambda p=path: self._reveal_file(p)).pack(side=tk.RIGHT, padx=4)

        ttk.Separator(self.inner).pack(fill=tk.X, padx=12, pady=2)

    def _load_thumb_async(self, path, label):
        if path in self.thumb_cache:
            label.configure(image=self.thumb_cache[path])
            return

        def load():
            try:
                with Image.open(path) as img:
                    img = img.convert("RGB")
                    img.thumbnail(THUMB_SIZE, Image.LANCZOS)
                    tk_img = ImageTk.PhotoImage(img)
                    self.thumb_cache[path] = tk_img
                    self.after(0, lambda: label.configure(image=tk_img))
            except Exception:
                self.after(0, lambda: label.configure(text="[no preview]"))

        threading.Thread(target=load, daemon=True).start()

    # ── selection helpers ──
    def _select_all_except_first(self):
        for i, v in enumerate(self.check_vars):
            v.set(i != 0)

    def _select_all_except_largest(self):
        group = self.groups[self.current_group]
        sizes = []
        for p in group:
            try:
                sizes.append(os.path.getsize(p))
            except OSError:
                sizes.append(0)
        largest = sizes.index(max(sizes))
        for i, v in enumerate(self.check_vars):
            v.set(i != largest)

    def _deselect_all(self):
        for v in self.check_vars:
            v.set(False)

    # ── actions ──
    def _get_selected_paths(self):
        group = self.groups[self.current_group]
        return [group[i] for i, v in enumerate(self.check_vars) if v.get()]

    def _trash_selected(self):
        paths = self._get_selected_paths()
        if not paths:
            messagebox.showinfo("Nothing selected", "Select files to move to trash first.")
            return
        if len(paths) == len(self.groups[self.current_group]):
            if not messagebox.askyesno("Warning", "You selected ALL files in this group. Continue?"):
                return
        confirm = messagebox.askyesno("Confirm", f"Move {len(paths)} file(s) to trash?")
        if not confirm:
            return
        errors = []
        for p in paths:
            try:
                send2trash(p)
            except Exception as e:
                errors.append(f"{p}: {e}")
        self._post_action(paths, errors, "trashed")

    def _delete_selected(self):
        paths = self._get_selected_paths()
        if not paths:
            messagebox.showinfo("Nothing selected", "Select files to delete first.")
            return
        if len(paths) == len(self.groups[self.current_group]):
            if not messagebox.askyesno("Warning", "You selected ALL files in this group. Continue?"):
                return
        confirm = messagebox.askyesno(
            "Permanent Delete",
            f"PERMANENTLY delete {len(paths)} file(s)?\nThis cannot be undone!",
        )
        if not confirm:
            return
        errors = []
        for p in paths:
            try:
                os.remove(p)
            except Exception as e:
                errors.append(f"{p}: {e}")
        self._post_action(paths, errors, "deleted")

    def _post_action(self, paths, errors, verb):
        if errors:
            messagebox.showerror("Errors", "\n".join(errors))
        succeeded = len(paths) - len(errors)
        self.status_var.set(f"{succeeded} file(s) {verb}.")
        # Remove successfully deleted files from group and refresh
        failed = {e.split(": ", 1)[0] for e in errors}
        removed = set(paths) - failed
        group = self.groups[self.current_group]
        self.groups[self.current_group] = [p for p in group if p not in removed]
        if len(self.groups[self.current_group]) < 2:
            # Group no longer has duplicates, remove it
            del self.groups[self.current_group]
            self.group_listbox.delete(0, tk.END)
            for i, g in enumerate(self.groups):
                self.group_listbox.insert(tk.END, f"Group {i + 1} ({len(g)} files)")
            if self.groups:
                idx = min(self.current_group, len(self.groups) - 1)
                self.group_listbox.selection_set(idx)
                self._show_group(idx)
            else:
                self._show_welcome()
                for w in self.inner.winfo_children():
                    w.destroy()
                ttk.Label(self.inner, text="All duplicates resolved!", font=("TkDefaultFont", 14), padding=40).pack()
        else:
            self._show_group(self.current_group)

    # ── platform file operations ──
    @staticmethod
    def _open_file(path):
        system = platform.system()
        if system == "Darwin":
            subprocess.Popen(["open", path])
        elif system == "Windows":
            os.startfile(path)
        else:
            subprocess.Popen(["xdg-open", path])

    @staticmethod
    def _reveal_file(path):
        system = platform.system()
        if system == "Darwin":
            subprocess.Popen(["open", "-R", path])
        elif system == "Windows":
            subprocess.Popen(["explorer", "/select,", path])
        else:
            subprocess.Popen(["xdg-open", os.path.dirname(path)])


def main():
    app = App()
    app.mainloop()


if __name__ == "__main__":
    main()
