# Web UI Polish Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix 8 web UI issues (URL encoding, responsive layout, hx-vals escaping, file preview context, SSE hardcoded JSON, charset, back-nav state, symbol color consistency) to prepare for public release.

**Architecture:** All changes are in `ferret-web/` — templates, CSS, JS, and a few Rust handler tweaks. No changes to core indexing logic or daemon protocol. Each task is independent and can be committed separately.

**Tech Stack:** Rust (askama templates, axum), HTML/CSS/JS (htmx), no new dependencies.

---

### Task 1: URL-encode query in pagination links

The pagination buttons in `search_results.html` interpolate `{{ query }}` directly into URL query strings. Askama HTML-escapes the value (e.g., `&` → `&amp;`) but does NOT URL-encode it. Queries with spaces, `+`, `&`, `#`, etc. produce broken pagination links.

**Files:**
- Modify: `ferret-web/src/ui.rs` (SearchResultsTemplate struct + impl)
- Modify: `ferret-web/templates/search_results.html:37,43`

**Step 1: Add a `query_encoded` field to `SearchResultsTemplate`**

In `ferret-web/src/ui.rs`, add a helper function and a new field:

```rust
/// Percent-encode a string for use in URL query values.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    out
}
```

Add `query_encoded: String` to `SearchResultsTemplate`. Populate it with `urlencode(&query)` everywhere `SearchResultsTemplate` is constructed (two places in `search_results_fragment`).

**Step 2: Update the pagination template to use the encoded field**

In `search_results.html`, change lines 37 and 43:

```html
<!-- line 37 -->
<button type="button" hx-get="/search-results?q={{ query_encoded }}&amp;repo-select={{ repo }}&amp;page={{ page - 1 }}" hx-target="#results">Prev</button>
<!-- line 43 -->
<button type="button" hx-get="/search-results?q={{ query_encoded }}&amp;repo-select={{ repo }}&amp;page={{ page + 1 }}" hx-target="#results">Next</button>
```

Note: `repo` names are validated to be simple identifiers, so they don't need encoding. `query_encoded` is already percent-encoded so askama's HTML-escaping of `%XX` sequences is harmless (no HTML-special chars in percent-encoded output).

**Step 3: Run tests**

```bash
cargo test -p ferret-indexer-web
cargo clippy -p ferret-indexer-web -- -D warnings
```

**Step 4: Commit**

```bash
git add ferret-web/src/ui.rs ferret-web/templates/search_results.html
git commit -m "fix(web): URL-encode query in pagination links"
```

---

### Task 2: Add responsive/mobile CSS

The sidebar is a fixed 220px and there are zero `@media` queries. On screens narrower than ~900px the UI is unusable.

**Files:**
- Modify: `ferret-web/static/style.css` (append responsive rules at the end)
- Modify: `ferret-web/static/app.js` (add sidebar toggle handler)
- Modify: `ferret-web/templates/base.html` (add hamburger button)

**Step 1: Add a sidebar toggle button to the header in `base.html`**

After the `<h1>ferret</h1>` line (line 14), add:

```html
<button class="sidebar-toggle" id="sidebar-toggle" type="button" aria-label="Toggle sidebar" aria-hidden="true">&#9776;</button>
```

The `aria-hidden="true"` is only on desktop; mobile CSS will reveal it.

**Step 2: Add responsive CSS at the end of `style.css`**

```css
/* Responsive: collapse sidebar on narrow viewports */
.sidebar-toggle {
    display: none;
    padding: 0.15rem 0.4rem;
    font-size: 1rem;
    background: none;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--fg);
    cursor: pointer;
    line-height: 1;
}

@media (max-width: 768px) {
    .sidebar-toggle {
        display: inline-flex;
        align-items: center;
    }

    .sidebar {
        position: fixed;
        top: var(--header-height);
        left: 0;
        bottom: 0;
        z-index: 50;
        transform: translateX(-100%);
        transition: transform 200ms ease;
        width: 260px;
        box-shadow: none;
    }

    .sidebar.sidebar--open {
        transform: translateX(0);
        box-shadow: var(--shadow-lg);
    }

    .file-header {
        flex-wrap: wrap;
    }

    .search-bar {
        padding: 0.5rem 0.75rem;
    }

    .stats-line {
        padding: 0.35rem 0.75rem;
        font-size: 0.75rem;
    }

    .file-header {
        padding: 0.4rem 0.75rem;
    }

    .line-content {
        padding: 0 0.4rem;
    }

    .repos-page {
        padding: 0.5rem 0.75rem 2rem;
    }

    .repo-card-stats {
        flex-wrap: wrap;
        gap: 0.75rem;
    }

    .breakdown-legend {
        flex-wrap: wrap;
        gap: 0.5rem;
    }
}
```

**Step 3: Add sidebar toggle JS to `app.js`**

Add this inside the IIFE, after the theme toggle block:

```javascript
// Mobile sidebar toggle
var sidebarToggle = document.getElementById("sidebar-toggle");
if (sidebarToggle) {
    sidebarToggle.addEventListener("click", function() {
        var sidebar = document.querySelector(".sidebar");
        if (sidebar) sidebar.classList.toggle("sidebar--open");
    });
    // Close sidebar when a repo is selected on mobile
    document.addEventListener("click", function(e) {
        if (e.target.closest(".sidebar-repo") && window.innerWidth <= 768) {
            var sidebar = document.querySelector(".sidebar");
            if (sidebar) sidebar.classList.remove("sidebar--open");
        }
    });
}
```

**Step 4: Run checks**

```bash
cargo clippy -p ferret-indexer-web -- -D warnings
```

**Step 5: Commit**

```bash
git add ferret-web/static/style.css ferret-web/static/app.js ferret-web/templates/base.html
git commit -m "feat(web): add responsive layout with collapsible sidebar"
```

---

### Task 3: Fix hx-vals JSON escaping for repo names

In `index.html:14`, `hx-vals='{"repo-select":"{{ repo.name }}"}'` breaks if a repo name contains `"` because askama HTML-escapes `"` to `&quot;`, the browser decodes it back to `"`, and htmx then parses invalid JSON.

**Files:**
- Modify: `ferret-web/templates/index.html:14`

**Step 1: Switch to hx-vals with JS expression syntax**

htmx supports `hx-vals="js:{...}"` which evaluates as JavaScript, avoiding the HTML entity round-trip. But the simplest robust fix is to move the repo name to a `data-` attribute and reference it from `hx-vals` with JS:

Actually, the cleanest fix: use `hx-vals` with the JS prefix:

In `index.html`, change line 14 from:

```html
hx-vals='{"repo-select":"{{ repo.name }}"}'>
```

to:

```html
hx-vals='js:{"repo-select": this.dataset.repo}'>
```

The `data-repo="{{ repo.name }}"` is already on the same element (line 11), and askama's HTML-escaping of the `data-repo` attribute value is correct for HTML attribute context.

**Step 2: Run checks**

```bash
cargo clippy -p ferret-indexer-web -- -D warnings
```

**Step 3: Commit**

```bash
git add ferret-web/templates/index.html
git commit -m "fix(web): use JS hx-vals to avoid JSON escaping issues with repo names"
```

---

### Task 4: Add repo context to file preview page

When navigating to a file from search results, the file preview page has no sidebar and no indication of which repo the file belongs to.

**Files:**
- Modify: `ferret-web/templates/file_preview.html`

**Step 1: Add repo name to the file preview header**

In `file_preview.html`, modify the header section. Change line 10 from:

```html
<h2>{{ path }}</h2>
```

to:

```html
<h2><span class="file-preview-repo">{{ repo }}</span> / {{ path }}</h2>
```

And update the back-link to include the repo:

```html
<a href="/" class="back-link">&larr; {{ repo }}</a>
```

**Step 2: Add minimal styling for the repo breadcrumb**

In `style.css`, add after the `.file-preview-header h2` rule:

```css
.file-preview-repo {
    color: var(--fg-muted);
    font-weight: 500;
}

.file-preview-repo::after {
    content: none;
}
```

**Step 3: Commit**

```bash
git add ferret-web/templates/file_preview.html ferret-web/static/style.css
git commit -m "feat(web): show repo name in file preview breadcrumb"
```

---

### Task 5: Replace hardcoded JSON strings in SSE status stream

`sse.rs` has three hardcoded JSON strings for fallback StatusResponse. If StatusResponse fields change, these go stale.

**Files:**
- Modify: `ferret-web/src/sse.rs:155-168`

**Step 1: Construct real StatusResponse objects and serialize them**

Replace the hardcoded strings with:

```rust
fn offline_status_json() -> String {
    serde_json::to_string(&ferret_indexer_daemon::StatusResponse {
        status: "offline".to_string(),
        files_indexed: 0,
        segments: 0,
        index_bytes: 0,
        last_indexed_ts: 0,
        languages: vec![],
        tombstone_ratio: 0.0,
        path_valid: true,
        tombstoned_count: 0,
        content_bytes: 0,
        trigrams_bytes: 0,
        meta_paths_bytes: 0,
        tombstones_bytes: 0,
        symbols_bytes: 0,
        segment_details: vec![],
        language_extensions: vec![],
        temp_bytes: 0,
        is_compacting: false,
    })
    .unwrap_or_else(|_| r#"{"status":"offline"}"#.to_string())
}
```

Then replace the three hardcoded strings at lines 156, 161, and 167 with calls to `offline_status_json()`. For the "unknown" case at line 156, use the same function but with `status: "unknown"` — or just use the actual payload from the daemon (which is already handled by the `unwrap_or_else`).

Actually, refactor to:

- Line 155-157: keep `result.payloads.into_iter().next().unwrap_or_else(|| offline_status_json())`
- Line 160-161: replace hardcoded string with `offline_status_json()`
- Line 166-167: replace hardcoded string with `offline_status_json()`

**Step 2: Run tests**

```bash
cargo test -p ferret-indexer-web
cargo clippy -p ferret-indexer-web -- -D warnings
```

**Step 3: Commit**

```bash
git add ferret-web/src/sse.rs
git commit -m "fix(web): replace hardcoded JSON in SSE status stream with typed construction"
```

---

### Task 6: Add charset=utf-8 for text MIME types in static file handler

`static_files.rs` serves text files without `charset=utf-8` in the Content-Type header.

**Files:**
- Modify: `ferret-web/src/static_files.rs:15`

**Step 1: Append charset for text/* MIME types**

Change the Content-Type line from:

```rust
(header::CONTENT_TYPE, mime.as_ref().to_string()),
```

to:

```rust
(header::CONTENT_TYPE, {
    let mime_str = mime.as_ref();
    if mime_str.starts_with("text/") {
        format!("{mime_str}; charset=utf-8")
    } else {
        mime_str.to_string()
    }
}),
```

**Step 2: Run tests**

```bash
cargo test -p ferret-indexer-web
cargo clippy -p ferret-indexer-web -- -D warnings
```

**Step 3: Commit**

```bash
git add ferret-web/src/static_files.rs
git commit -m "fix(web): add charset=utf-8 to text/* static file responses"
```

---

### Task 7: Preserve search state across back-navigation

When navigating to file preview and pressing back, the search query and results are lost.

**Files:**
- Modify: `ferret-web/templates/index.html` (add `hx-replace-url` to search input)
- Modify: `ferret-web/static/app.js` (restore query from URL on page load)

**Step 1: Add `hx-replace-url="true"` to the search input**

In `index.html`, add `hx-replace-url="true"` to the search input element (line 37). This makes htmx update the browser URL as search results stream in, so the back button returns to a URL with the query param.

Change the `<input type="search"...>` attributes to include:

```html
hx-replace-url="true"
```

Note: this goes on the search input alongside the existing `hx-get`, `hx-trigger`, etc. htmx will update the URL to `/search-results?q=...&repo-select=...&mode=...` after each swap. But we actually want the URL to stay at `/` with query params. So instead, we'll handle this in JS.

**Alternative approach — JS-based URL state:**

Don't add `hx-replace-url`. Instead, add JS that:
1. After each htmx swap of search results, updates `history.replaceState` with the current query
2. On page load, reads the URL params and restores the search

In `app.js`, add after the `htmx:afterSwap` handler:

```javascript
// Preserve search state in URL for back-navigation
document.addEventListener("htmx:afterSwap", function(e) {
    if (e.target && e.target.id === "results") {
        var input = document.querySelector(".search-input");
        var radio = document.querySelector(".sidebar-repo-radio:checked");
        var mode = document.getElementById("search-mode");
        if (input) {
            var params = new URLSearchParams();
            if (input.value) params.set("q", input.value);
            if (radio) params.set("repo", radio.value);
            if (mode && mode.value !== "text") params.set("mode", mode.value);
            var url = params.toString() ? "/?" + params.toString() : "/";
            history.replaceState(null, "", url);
        }
    }
});

// Restore search state from URL on page load
(function() {
    var params = new URLSearchParams(window.location.search);
    var q = params.get("q");
    var repo = params.get("repo");
    var mode = params.get("mode");
    if (!q) return;
    var input = document.querySelector(".search-input");
    if (!input) return;
    input.value = q;
    if (repo) {
        var radio = document.querySelector('.sidebar-repo-radio[value="' + CSS.escape(repo) + '"]');
        if (radio) {
            radio.checked = true;
            var repoDiv = radio.closest(".sidebar-repo");
            if (repoDiv) {
                document.querySelectorAll(".sidebar-repo").forEach(function(el) { el.classList.remove("sidebar-repo--active"); });
                repoDiv.classList.add("sidebar-repo--active");
            }
        }
    }
    if (mode && mode === "symbol") {
        var modeInput = document.getElementById("search-mode");
        if (modeInput) modeInput.value = mode;
        var symBtn = document.getElementById("mode-symbol");
        var textBtn = document.getElementById("mode-text");
        if (symBtn) symBtn.classList.add("mode-btn--active");
        if (textBtn) textBtn.classList.remove("mode-btn--active");
    }
    // Trigger the search
    htmx.trigger(input, "search");
})();
```

**Step 2: Run checks**

```bash
cargo clippy -p ferret-indexer-web -- -D warnings
```

**Step 3: Commit**

```bash
git add ferret-web/static/app.js ferret-web/templates/index.html
git commit -m "feat(web): preserve search query across back-navigation via URL state"
```

---

### Task 8: Align symbol kind badge colors with syntax token colors

Symbol kind badges (`.symbol-kind--fn`) use hardcoded blues that differ from the syntax token CSS variables (`--tok-function`). They should use the same palette.

**Files:**
- Modify: `ferret-web/static/style.css` (symbol kind rules, ~lines 1377-1390)

**Step 1: Update light-mode symbol kind colors to reference token variables**

Replace the symbol kind color rules (lines 1377-1383) with:

```css
.symbol-kind--fn, .symbol-kind--method { color: var(--tok-function); border-color: color-mix(in srgb, var(--tok-function) 40%, transparent); background: color-mix(in srgb, var(--tok-function) 8%, transparent); }
.symbol-kind--struct, .symbol-kind--class { color: var(--tok-type); border-color: color-mix(in srgb, var(--tok-type) 40%, transparent); background: color-mix(in srgb, var(--tok-type) 8%, transparent); }
.symbol-kind--trait, .symbol-kind--interface { color: var(--tok-macro); border-color: color-mix(in srgb, var(--tok-macro) 40%, transparent); background: color-mix(in srgb, var(--tok-macro) 8%, transparent); }
.symbol-kind--enum { color: var(--tok-number); border-color: color-mix(in srgb, var(--tok-number) 40%, transparent); background: color-mix(in srgb, var(--tok-number) 8%, transparent); }
.symbol-kind--const { color: var(--tok-constant); border-color: color-mix(in srgb, var(--tok-constant) 40%, transparent); background: color-mix(in srgb, var(--tok-constant) 8%, transparent); }
.symbol-kind--var, .symbol-kind--mod { color: var(--tok-module); }
.symbol-kind--type { color: var(--tok-type); border-color: color-mix(in srgb, var(--tok-type) 40%, transparent); background: color-mix(in srgb, var(--tok-type) 8%, transparent); }
```

**Step 2: Remove the dark-mode overrides (lines 1385-1390)**

Since the colors now use CSS variables that already have dark-mode values defined in the `[data-theme="dark"]` root block, the per-selector dark-mode overrides are no longer needed. Delete lines 1385-1390 entirely.

**Step 3: Commit**

```bash
git add ferret-web/static/style.css
git commit -m "fix(web): align symbol kind badge colors with syntax token palette"
```

---

## Execution Order

Tasks are independent — they touch different files or different sections. Recommended order for cleanest diffs:

1. Task 1 (URL encoding) — most impactful bug fix
2. Task 3 (hx-vals escaping) — related template fix
3. Task 5 (SSE hardcoded JSON) — small Rust fix
4. Task 6 (charset) — small Rust fix
5. Task 4 (file preview context) — template + CSS
6. Task 8 (symbol colors) — CSS only
7. Task 2 (responsive layout) — CSS + JS + template
8. Task 7 (back-nav state) — JS only, most complex
