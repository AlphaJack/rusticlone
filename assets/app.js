/** @file Behavior that htmx attributes cannot express. A module, so that its names stay private; runs after htmx, once the page is parsed. */

/**
 * The page's dialog for forms, outside the part the server updates. Looked up on each use, because a
 * page opened through `HX-Location` brings its own.
 * @returns {HTMLDialogElement}
 */
const dialog = () => document.getElementById("dialog");

const close = () => dialog()?.open && dialog().close();
// empty it once closed; `close` does not bubble, so listen in the capture phase
document.addEventListener("close", (event) => event.target === dialog() && dialog().replaceChildren(), true);
// a click on the backdrop closes it; the backdrop counts as the dialog itself
document.addEventListener("click", (event) => {
  const box = dialog()?.getBoundingClientRect();
  if (!box || event.target !== dialog()) return;
  const inside = event.clientX >= box.left && event.clientX <= box.right && event.clientY >= box.top && event.clientY <= box.bottom;
  if (!inside) close();
});
// htmx 4 reports the requesting element, not the swap target
document.addEventListener("htmx:after:swap", (event) => {
  const d = dialog();
  if (event.detail.ctx.target === d && d.childElementCount && !d.open) d.showModal();
});

// a form with [data-close-on-success] closes the dialog once the server accepts it
document.addEventListener("htmx:finally:request", (event) => {
  const form = event.target.closest?.("[data-close-on-success]");
  if (form && event.detail.ctx.response?.status < 400) close();
});

// `serve --open` sends the token in the fragment, which stays in the browser
const login = document.querySelector("form.login");
if (login && location.hash.length > 1) {
  login.elements.token.value = decodeURIComponent(location.hash.slice(1));
  history.replaceState(null, "", location.pathname + location.search);
  login.submit();
}
// The empty fragment that replaced it, when already logged in.
if (location.href.endsWith("#")) history.replaceState(null, "", location.pathname + location.search);

// A notification toggle adds its URL to the field, or removes every URL of its schemes, even edited.
document.addEventListener("change", (event) => {
  const toggle = event.target;
  if (!toggle.dataset.schemes) return;
  const field = toggle.form.elements.notify;
  const urls = urlsOf(field.value).filter((url) => !ofSchemes(url, toggle.dataset.schemes));
  field.value = [...urls, ...(toggle.checked ? [toggle.dataset.template] : [])].join(" ");
});
// Typed URLs turn the toggles of their schemes on, and deleted ones off.
document.addEventListener("input", (event) => {
  if (event.target.name !== "notify") return;
  const urls = urlsOf(event.target.value);
  for (const toggle of event.target.form.querySelectorAll("[data-schemes]")) toggle.checked = urls.some((url) => ofSchemes(url, toggle.dataset.schemes));
});
/** As mercurio splits them: at spaces, commas or semicolons before a `scheme://`, so a phone number keeps its spaces. @param {string} text */
const urlsOf = (text) => text.split(/[\s,;]+(?=[a-z0-9]+:\/\/)/i).map((url) => url.trim()).filter(Boolean);
/** @param {string} url @param {string} schemes space-separated */
const ofSchemes = (url, schemes) => url.includes("://") && schemes.split(" ").includes(url.split("://")[0].toLowerCase());

/** Whether `role`'s repository forgets backups: it is chosen and does not refuse deletes. @param {HTMLFormElement} form @param {string} role */
const forgets = (form, role) => {
  const kind = form.elements[role]?.value;
  return Boolean(kind) && kind !== "none" && !form.elements[`${role}.${kind}.immutable`]?.checked;
};

/** Shows the fields of each choice made in `form`, and how long backups are kept while a repository forgets them. @param {HTMLFormElement} form */
const showChosen = (form) => {
  for (const fields of form.querySelectorAll("[data-choice]")) {
    const chosen = form.elements[fields.dataset.choice];
    if (chosen) fields.hidden = fields.dataset.value !== chosen.value;
  }
  const keep = form.querySelector("[data-keep]");
  if (keep) keep.hidden = !["primary", "target"].some((role) => forgets(form, role));
};
document.addEventListener("change", (event) => event.target.form && showChosen(event.target.form));
// Firefox fills a reloaded form with the choices made before, without a change event.
window.addEventListener("pageshow", () => [...document.forms].forEach(showChosen));

/**
 * Warns where the folders to back up, as far as measured, are larger than the free space. Compression and
 * deduplication may still make them fit. @param {HTMLFormElement | null} form
 */
const compareSizes = (form) => {
  if (!form) return;
  const total = [...form.querySelectorAll(".measured [data-bytes]")].reduce((sum, size) => sum + Number(size.dataset.bytes), 0);
  for (const free of form.querySelectorAll(".free [data-bytes]")) {
    const short = free.parentElement.querySelector("[data-short]");
    if (short) short.hidden = total <= Number(free.dataset.bytes);
  }
};
document.addEventListener("htmx:after:swap", (event) => compareSizes(event.target.closest?.("form")));

// Folder rows: Add appends an empty one, Remove drops one but the first. Their fields are numbered again, as
// the server reads them in order.
document.addEventListener("click", (event) => {
  const list = event.target.closest(".sources");
  const add = event.target.closest("[data-add]");
  const remove = event.target.closest("[data-remove]");
  if (!list || !(add || remove)) return;
  const rows = () => [...list.querySelectorAll(".source")];
  if (add) {
    const row = rows().at(-1).cloneNode(true);
    row.querySelector("input").value = "";
    row.querySelector(".measured")?.replaceChildren();
    rows().at(-1).after(row);
    // The clone's size request is wired like the original's.
    htmx.process(row);
  } else {
    remove.closest(".source").remove();
    compareSizes(list.closest("form"));
  }
  rows().forEach((row, i) => {
    Object.assign(row.querySelector("input"), { name: `${list.dataset.field}.${i}`, id: `setup-${list.dataset.field}-${i}` });
  });
  if (add) rows().at(-1).querySelector("input").focus();
});

/**
 * Keeps all spinners in phase. Each one that starts, from a swap, a live update or a hover that shows
 * it again, joins the others.
 */
const spin = () => {
  for (const animation of document.getAnimations()) if (animation.animationName === "spin") animation.startTime = 0;
};
spin();
document.addEventListener("animationstart", (event) => event.animationName === "spin" && spin());

/**
 * Whether each log was scrolled to its end before a swap. Logs follow new lines unless scrolled up.
 * @type {WeakMap<HTMLElement, boolean>}
 */
const following = new WeakMap();

/**
 * @param {HTMLElement} log
 * @returns {boolean} whether it is scrolled to its end, or nearly
 */
const atEnd = (log) => log.scrollTop + log.clientHeight >= log.scrollHeight - 40;

/** @returns {NodeListOf<HTMLPreElement>} */
const logs = () => document.querySelectorAll("pre.log");
document.addEventListener("htmx:before:swap", () => {
  for (const log of logs()) following.set(log, atEnd(log));
});
document.addEventListener("htmx:after:swap", () => {
  for (const log of logs()) if (following.get(log) ?? true) log.scrollTop = log.scrollHeight;
});
for (const log of logs()) log.scrollTop = log.scrollHeight;

/**
 * Copies text. Falls back to a hidden selection where the Clipboard API is missing: it needs HTTPS
 * or localhost, and a reverse proxy may serve plain HTTP.
 * @param {string} text
 */
const copy = async (text) => {
  if (navigator.clipboard?.writeText) return navigator.clipboard.writeText(text);
  const input = document.createElement("textarea");
  input.value = text;
  input.style.position = "fixed";
  input.style.opacity = "0";
  document.body.append(input);
  try {
    input.select();
    if (!document.execCommand("copy")) throw new Error();
  } finally {
    input.remove();
  }
};

/**
 * Shows a toast, like the server's (`flash.html`).
 * @param {"success" | "warning" | "failure"} level
 * @param {string} message
 */
const toast = (level, message) => {
  const flash = document.createElement("div");
  flash.className = `flash ${level}`;
  flash.textContent = message;
  document.getElementById("flash").replaceChildren(flash);
};

/** `data-copy` copies its value; `data-copy-log` copies the card's current log. */
document.addEventListener("click", async (event) => {
  const button = event.target.closest("[data-copy], [data-copy-log]");
  if (!button) return;
  const log = button.closest(".card").querySelector("pre.log");
  try {
    await copy(button.dataset.copy ?? log.textContent);
    button.focus({ preventScroll: true });
    toast("success", document.body.dataset.copied);
  } catch {
    if (button.dataset.copy !== undefined) return toast("failure", document.body.dataset.cannotCopy);
    const range = document.createRange();
    range.selectNodeContents(log);
    const selection = window.getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
    toast("warning", document.body.dataset.logSelected);
  }
});

/**
 * The footer's dot shows whether the page receives live updates, or lost them (server restarted,
 * network down) while htmx reconnects. Only the color changes. A state is shown only after a delay,
 * so that opening a page or leaving one does not make it blink.
 */
const UNTOLD_MS = 3000;
/** @type {ReturnType<typeof setTimeout> | undefined} */
let untold;
/** @type {"connecting" | "lost"} */
let pending = "connecting";
/** @param {"live" | "connecting" | "lost"} state */
const show = (state) => {
  const dot = document.getElementById("live");
  if (!dot) return;
  dot.className = { live: "dot", connecting: "dot warning", lost: "dot failure" }[state];
  // The page names each state in its language: `data-live`, `data-connecting`, `data-lost`.
  dot.title = dot.dataset[state];
  dot.firstElementChild.textContent = `${dot.title} `;
};
/** @param {"connecting" | "lost"} state */
const unlessLive = (state) => {
  pending = state;
  untold ??= setTimeout(() => show(pending), UNTOLD_MS);
};
unlessLive("connecting");
document.addEventListener("htmx:sse:error", () => unlessLive("lost"));
document.addEventListener("htmx:sse:after:connection", () => {
  clearTimeout(untold);
  untold = undefined;
  show("live");
});

// why a run started from this page failed, on whichever page is open
document.addEventListener("flash", (event) => {
  document.getElementById("flash").innerHTML = event.detail.data;
});

// new log lines arrive separately, after the page, which keeps the lines it has
document.addEventListener("log", (event) => {
  const log = document.querySelector("pre.log");
  if (!log) return;
  const follow = atEnd(log);
  log.insertAdjacentHTML("beforeend", event.detail.data);
  if (follow) log.scrollTop = log.scrollHeight;
});

/**
 * A step or command under the pointer lights up its commands and log lines.
 * @param {Event} event
 * @param {boolean} on
 */
const light = (event, on) => {
  const row = event.target.closest?.("#log tr[data-step], #log tr[data-call]");
  if (!row) return;
  const { call, step } = row.dataset;
  const lit = call === undefined ? `[data-step="${step}"]` : `[data-call="${call}"]`;
  for (const e of row.closest(".card").querySelectorAll(lit)) e.classList.toggle("lit", on);
};
document.addEventListener("mouseover", (event) => light(event, true));
// clicking a step or a command scrolls the log to its first line, if the log still shows it
document.addEventListener("click", (event) => {
  const row = event.target.closest?.("#log td.jump")?.parentElement;
  if (!row) return;
  const { call, step } = row.dataset;
  const log = row.closest(".card").querySelector("pre.log");
  const first = log?.querySelector(call === undefined ? `[data-step="${step}"]` : `[data-call="${call}"]`);
  if (first) log.scrollTop = first.offsetTop - log.offsetTop;
});
document.addEventListener("mouseout", (event) => light(event, false));

/**
 * The sort order of each table, by its `data-sort`: the column's index and whether descending. Kept
 * across page updates and in the URL fragment (`#repos=3-desc&runs=0`), so that a reload or a shared
 * link sorts the same way. An anchor (`#log`), or a table the page lacks, is no order.
 * @type {Map<string, [number, boolean]>}
 */
const orders = new Map(
  [...new URLSearchParams(location.hash.slice(1))]
    .filter(([name, order]) => order && document.querySelector(`table[data-sort="${CSS.escape(name)}"]`))
    .map(([name, order]) => [name, [Number.parseInt(order, 10), order.endsWith("-desc")]]),
);
const collator = new Intl.Collator(undefined, { numeric: true });
/**
 * The sort key of a cell: a time's timestamp, a value's machine-readable form, else its text.
 * @param {HTMLTableCellElement} cell
 * @returns {string}
 */
const sortKey = (cell) => cell.querySelector("time")?.dateTime ?? cell.querySelector("data")?.value ?? cell.textContent.trim();

/**
 * Its rows, or its row groups (one `<tbody class="group">` each), keyed by their first row.
 * @param {HTMLTableElement} table
 */
const sort = (table) => {
  const [column, down] = orders.get(table.dataset.sort) ?? [];
  const headers = [...table.tHead.rows[0].cells];
  // a column from a link created when the table still had it
  if (!headers[column]?.textContent.trim()) return;
  const grouped = table.tBodies[0].classList.contains("group");
  const parent = grouped ? table : table.tBodies[0];
  const items = grouped ? [...table.tBodies] : [...parent.rows];
  const key = (item) => sortKey((grouped ? item.rows[0] : item).cells[column]);
  items.sort((a, b) => collator.compare(key(a), key(b)) * (down ? -1 : 1));
  parent.append(...items);
  for (const th of headers) th.removeAttribute("aria-sort");
  headers[column].setAttribute("aria-sort", down ? "descending" : "ascending");
};

// clicking a header sorts by its column; clicking again reverses the order
document.addEventListener("click", (event) => {
  const th = event.target.closest("table[data-sort] th");
  if (!th?.textContent.trim()) return;
  const table = th.closest("table");
  const [column, down] = orders.get(table.dataset.sort) ?? [-1, true];
  orders.set(table.dataset.sort, [th.cellIndex, th.cellIndex === column && !down]);
  const fragment = [...orders].map(([table, [column, down]]) => [table, down ? `${column}-desc` : `${column}`]);
  history.replaceState(null, "", `#${new URLSearchParams(fragment)}`);
  sort(table);
});
const sortAll = () => document.querySelectorAll("table[data-sort]").forEach(sort);
sortAll();
document.addEventListener("htmx:after:swap", sortAll);
