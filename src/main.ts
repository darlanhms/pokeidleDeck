import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { icons } from "./icons";

type Status = "starting" | "loading" | "ok" | "off";
type Rect = { x: number; y: number; w: number; h: number };
type Pane = {
  id: string;
  name: string;
  off: boolean;
  muted: boolean;
  status: Status;
  startedAt: number | null;
  focused: boolean;
  restockOn: boolean;
  rect: Rect | null;
};
type LayoutMode = "grid" | "columns" | "focus";
type DeckView = {
  panes: Pane[];
  layout: LayoutMode;
  zoom: number;
  stacked: boolean;
  canAdd: boolean;
  removed: number;
  fullscreen: boolean;
  ultra: boolean;
  reduceMotion: boolean;
  echo: boolean;
  header: number;
  topBar: number;
  bottomBar: number;
};
type Restock = { configured: boolean; on: boolean; connected: boolean; paused?: boolean; gold?: number; log: string[] };
type Report = {
  gold: number | null;
  nick: string | null;
  connected: boolean;
  login: boolean;
  url: string;
  restock: Restock | null;
};
type Memory = { game: number; lives: number; other: number };
type LivesStatus = {
  on: boolean;
  playing: number;
  open: number;
  redeemable: number;
  text: string;
  accounts: { id: string; playing: number; redeemable: number; warn: boolean }[];
};
type KickSnapshot = {
  channel: string;
  elapsedSecs: number;
  players: { key: string; playing: boolean | null; checks: number; stoppedChecks: number; detail: string }[];
};

const ZOOM_MIN = 0.5;
const ZOOM_MAX = 1.5;
const NOTE_MS = 4000;
// Abaixo disto o cabeçalho não tem lugar para os chips: ouro e tempo vão para a dica do nome.
const NARROW_HEADER = 560;

const $ = <T extends HTMLElement>(selector: string) => document.querySelector<T>(selector)!;
const reports = new Map<string, Report>();
const notes = new Map<string, { text: string; until: number }>();
const compact = new Intl.NumberFormat("pt-BR", { notation: "compact", maximumFractionDigits: 1 });
let view: DeckView | null = null;
let livesStatus: LivesStatus | null = null;
let dragging: { id: string; x: number; y: number; active: boolean } | null = null;

const action = (name: string) => invoke("deck_action", { action: name });
const accountMenu = (id: string) => invoke("account_menu", { id });
const popupBelow = (command: string, anchor: HTMLElement) => {
  const rect = anchor.getBoundingClientRect();
  invoke(command, { x: rect.left, y: rect.bottom });
};

function el<K extends keyof HTMLElementTagNameMap>(tag: K, props: Partial<HTMLElementTagNameMap[K]> = {}, ...children: (Node | string)[]) {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

function iconNode(svg: string): Node {
  const template = document.createElement("template");
  template.innerHTML = svg;
  return template.content.firstChild!;
}

function uptime(since: number): string {
  const minutes = Math.floor((Date.now() - since) / 60000);
  return minutes >= 60 ? `${Math.floor(minutes / 60)} h ${String(minutes % 60).padStart(2, "0")}` : `${minutes} min`;
}

function gold(pane: Pane): string | null {
  const report = reports.get(pane.id);
  return report?.connected && report.gold !== null ? compact.format(report.gold) : null;
}

function activeNote(id: string): string | null {
  const note = notes.get(id);
  return note && note.until > Date.now() ? note.text : null;
}

function dotClass(pane: Pane): string {
  if (pane.off) return "";
  if (reports.get(pane.id)?.login) return "login";
  return pane.status === "ok" ? "ok" : "loading";
}

/** O que o carrinho mostra: desligada, comprando, esperando o jogo ou em pausa. */
function restockState(pane: Pane): { className: string; label: string; title: string } {
  const restock = reports.get(pane.id)?.restock;
  const last = restock?.log.length ? `\n${restock.log[restock.log.length - 1]}` : "";
  if (!pane.restockOn) {
    return { className: "", label: "Recompra", title: "Recompra desligada: clique para configurar e ligar (pokébolas, revives e poções no Market do NPC)" };
  }
  if (!restock?.connected) return { className: "waiting", label: "Esperando", title: "Recompra ligada: esperando o jogo conectar" };
  if (restock.paused) return { className: "waiting", label: "Em pausa", title: `Recompra em pausa: 3 compras seguidas não deram certo (volta sozinha em 2 min)${last}` };
  return { className: "on", label: "Comprando", title: `Recompra ligada${last}` };
}

function actionButton(icon: string, label: string, onClick: (e: MouseEvent) => void, className = "") {
  const button = el("button", { type: "button", title: label, ariaLabel: label, className });
  button.append(iconNode(icon));
  button.addEventListener("click", (e) => {
    e.stopPropagation();
    onClick(e);
  });
  button.addEventListener("dblclick", (e) => e.stopPropagation());
  return button;
}

function onContextMenu(node: HTMLElement, id: string) {
  node.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    accountMenu(id);
  });
}

function renderTop(v: DeckView) {
  $("#top").style.height = `${v.topBar}px`;
  $("#bottom").style.height = `${v.bottomBar}px`;
  for (const button of document.querySelectorAll<HTMLButtonElement>(".layouts button")) {
    const active = button.dataset.layout === v.layout;
    button.classList.toggle("active", active);
    button.ariaPressed = String(active);
  }

  const focus = v.layout === "focus";
  const tabs = $("#tabs");
  tabs.hidden = !focus;
  $("#add").hidden = focus;
  $<HTMLButtonElement>("#add").disabled = !v.canAdd;
  $("#add").title = v.canAdd ? "Nova conta (⌘N)" : "O deck já tem 4 contas";
  if (focus) {
    tabs.replaceChildren(
      ...v.panes.map((pane, index) => {
        const detail = pane.off ? "desligada" : reports.get(pane.id)?.login ? "login" : gold(pane) ?? (pane.status === "ok" ? "" : "carregando…");
        const tab = el(
          "button",
          { type: "button", className: pane.focused ? "current" : "", title: `Ampliar ${pane.name} (⌘⌥${index + 1})` },
          el("span", { className: `dot ${dotClass(pane)}` }),
          el("span", { className: "tab-name", textContent: pane.name }),
          el("span", { className: "tab-detail", textContent: detail }),
        );
        if (pane.focused) tab.ariaCurrent = "page";
        tab.dataset.id = pane.id;
        tab.addEventListener("click", () => action(`deck:focus:${index + 1}`));
        onContextMenu(tab, pane.id);
        return tab;
      }),
    );
  }

  const pill = (id: string, on: boolean, text: string) => {
    const button = $(id);
    button.classList.toggle("on", on);
    button.ariaPressed = String(on);
    button.lastChild!.textContent = text;
  };
  pill("#echo", v.echo, v.echo ? "Eco ligado" : "Eco");
  pill("#ultra", v.ultra, v.ultra ? "ULTRA ligado" : "ULTRA");
  $("#ultra-menu").classList.toggle("on", v.ultra);

  $<HTMLButtonElement>("#zoom-menu").textContent = `${Math.round(v.zoom * 100)}%`;
  $<HTMLButtonElement>("#zoom-out").disabled = v.zoom <= ZOOM_MIN + 0.001;
  $<HTMLButtonElement>("#zoom-in").disabled = v.zoom >= ZOOM_MAX - 0.001;
  const fullscreen = $<HTMLButtonElement>("#fullscreen");
  fullscreen.replaceChildren(iconNode(v.fullscreen ? icons.exitFullscreen : icons.fullscreen));
  fullscreen.title = v.fullscreen ? "Sair da tela cheia (F11)" : "Tela cheia (F11)";
  fullscreen.ariaLabel = fullscreen.title;
}

function renderHeader(v: DeckView, pane: Pane, r: Rect): HTMLElement {
  const narrow = r.w < NARROW_HEADER;
  const coins = gold(pane);
  const name = el("span", { className: "name", textContent: pane.name });
  const details = [coins && `ouro ${coins}`, pane.startedAt && `rodando há ${uptime(pane.startedAt)}`].filter(Boolean).join(" · ");
  name.title = `${narrow && details ? `${details}\n` : ""}Arraste até o cabeçalho de outra conta para trocar de lugar`;
  const header = el("div", { className: "pane-header" }, el("span", { className: `dot ${dotClass(pane)}` }), name);

  const note = activeNote(pane.id);
  const warning = note ?? (!pane.off && reports.get(pane.id)?.login ? "Na tela de login: entre ou desligue" : null);
  if (warning) {
    header.append(el("span", { className: "chip warn", title: warning }, iconNode(icons.alert), warning));
  } else if (pane.off) {
    header.append(el("span", { className: "status-text", textContent: "desligada" }));
  } else if (pane.status !== "ok") {
    header.append(el("span", { className: "status-text", textContent: pane.status === "starting" ? "abrindo…" : "carregando…" }));
  } else {
    const chips = el("div", { className: "chips" });
    if (!narrow && coins) chips.append(el("span", { className: "chip gold", title: "Ouro da conta" }, iconNode(icons.coins), coins));
    if (!narrow && pane.startedAt) chips.append(el("span", { className: "chip muted", title: "Rodando há" }, iconNode(icons.clock), uptime(pane.startedAt)));
    const lives = livesStatus?.accounts.find((a) => a.id === pane.id);
    if (lives) {
      const extra = lives.redeemable > 0 ? ` · ${lives.redeemable} ${lives.redeemable === 1 ? "resgate" : "resgates"}` : lives.warn ? " · entrar" : "";
      const title = `Lives: ${lives.playing} tocando${lives.redeemable > 0 ? `; ${lives.redeemable} canais prontos para resgatar` : ""}${lives.warn ? "; falta entrar na Twitch ou na Kick" : ""}. Clique para abrir a central das lives.`;
      const chip = actionButton(icons.tv, title, () => action(`acc:lives:${pane.id}`), `chip ${lives.redeemable > 0 ? "accent" : lives.warn ? "waiting" : ""} ${narrow ? "icon-only" : ""}`);
      if (!narrow) chip.append(`Lives ▶${lives.playing}${extra}`);
      chips.append(chip);
    }
    const restock = restockState(pane);
    const cart = actionButton(icons.cart, restock.title, () => action(`acc:restock:${pane.id}`), `chip ${restock.className} ${narrow ? "icon-only" : ""}`);
    if (!narrow) cart.append(restock.label);
    chips.append(cart);
    header.append(chips);
  }

  const actions = el("div", { className: "actions" });
  if (!pane.off) {
    actions.append(
      actionButton(pane.muted ? icons.muted : icons.sound, pane.muted ? "Ligar o som desta conta" : "Desligar o som desta conta", () => action(`acc:mute:${pane.id}`)),
      actionButton(icons.reload, "Recarregar a página", () => action(`acc:reload:${pane.id}`)),
    );
  }
  actions.append(
    actionButton(
      pane.focused ? icons.shrink : icons.expand,
      pane.focused ? "Voltar ao layout de antes (duplo clique no cabeçalho faz o mesmo)" : "Ampliar esta conta (duplo clique no cabeçalho faz o mesmo)",
      () => action(`acc:focus:${pane.id}`),
      pane.focused ? "current" : "",
    ),
    actionButton(
      icons.power,
      pane.off ? "Ligar esta conta: o jogo abre de novo" : "Desligar esta conta: o jogo dela para (ela continua aqui no deck)",
      () => action(`acc:power:${pane.id}`),
      `power ${pane.off ? "off" : ""}`,
    ),
    actionButton(icons.more, "Mais opções: renomear, recompra, trocar de lugar, remover…", () => accountMenu(pane.id)),
  );
  header.append(actions);

  header.dataset.id = pane.id;
  makeDraggable(header, pane.id);
  Object.assign(header.style, { left: `${r.x}px`, top: `${r.y}px`, width: `${r.w}px`, height: `${v.header}px` });
  header.addEventListener("dblclick", () => action(`acc:focus:${pane.id}`));
  onContextMenu(header, pane.id);
  return header;
}

function renderOffPane(v: DeckView, pane: Pane, r: Rect): HTMLElement {
  const turnOn = el("button", { type: "button", className: "primary", textContent: "Ligar conta" });
  turnOn.addEventListener("click", () => action(`acc:power:${pane.id}`));
  const remove = el("button", { type: "button", className: "ghost", textContent: "Remover…", disabled: v.panes.length <= 1 });
  remove.addEventListener("click", () => action(`acc:remove:${pane.id}`));
  const body = el(
    "div",
    { className: "off-pane" },
    iconNode(icons.power),
    el("strong", { textContent: `${pane.name} está desligada` }),
    el("span", { textContent: "Não roda nada e não gasta memória." }),
    el("div", { className: "buttons" }, turnOn, remove),
  );
  body.addEventListener("dblclick", () => action(`acc:focus:${pane.id}`));
  Object.assign(body.style, {
    left: `${r.x}px`,
    top: `${r.y + v.header}px`,
    width: `${r.w}px`,
    height: `${Math.max(0, r.h - v.header)}px`,
  });
  return body;
}

function renderPanes(v: DeckView) {
  const nodes: HTMLElement[] = [];
  for (const pane of v.panes) {
    if (!pane.rect) continue;
    nodes.push(renderHeader(v, pane, pane.rect));
    if (pane.off) nodes.push(renderOffPane(v, pane, pane.rect));
  }
  $("#panes").replaceChildren(...nodes);
}

/** A compra mais recente entre as contas, pelo registro da recompra ("14:02:11 comprou …"). */
function lastPurchase(v: DeckView): { pane: Pane; text: string; at: number } | null {
  let best: { pane: Pane; text: string; at: number } | null = null;
  const today = new Date();
  for (const pane of v.panes) {
    const line = [...(reports.get(pane.id)?.restock?.log ?? [])].reverse().find((l) => l.includes(" comprou "));
    const match = line && /^(\d{1,2}):(\d{2}):(\d{2}) (.*)$/.exec(line);
    if (!match) continue;
    const at = new Date(today.getFullYear(), today.getMonth(), today.getDate(), +match[1], +match[2], +match[3]).getTime();
    if (at > Date.now() + 60_000) continue;
    if (!best || at > best.at) best = { pane, text: match[4].replace(/ \(−[^)]*\)$/, ""), at };
  }
  return best;
}

function ago(at: number): string {
  const seconds = Math.max(0, Math.round((Date.now() - at) / 1000));
  if (seconds < 60) return `há ${seconds} s`;
  const minutes = Math.round(seconds / 60);
  return minutes < 60 ? `há ${minutes} min` : `há ${Math.floor(minutes / 60)} h ${String(minutes % 60).padStart(2, "0")}`;
}

function renderBottom(v: DeckView) {
  const on = v.panes.filter((p) => !p.off).length;
  $("#summary").textContent = `${v.panes.length} ${v.panes.length === 1 ? "conta" : "contas"} · ${on} ${on === 1 ? "ligada" : "ligadas"}`;
  const buy = lastPurchase(v);
  const button = $<HTMLButtonElement>("#last-buy");
  button.hidden = !buy;
  if (buy) {
    button.textContent = `${buy.pane.name} ${buy.text} · ${ago(buy.at)}`;
    button.title = `Abrir a recompra de ${buy.pane.name}`;
    button.dataset.id = buy.pane.id;
  }
}

const gb = (bytes: number) => (bytes / 1024 ** 3).toLocaleString("pt-BR", { minimumFractionDigits: 1, maximumFractionDigits: 1 });

function renderMemory(m: Memory) {
  const memory = $("#memory");
  memory.replaceChildren("RAM ", el("strong", { textContent: `${gb(m.game + m.lives + m.other)} GB` }), ` jogo ${gb(m.game)} · lives ${gb(m.lives)} · navegador ${gb(m.other)}`);
}

// Arrastar um cabeçalho (ou uma aba) até o de outra conta troca as duas de lugar. Só dá para soltar em
// cabeçalhos e abas: a página do deck não desenha por cima das contas.
function dropTarget(x: number, y: number, source: string): HTMLElement | null {
  const target = document.elementFromPoint(x, y)?.closest<HTMLElement>("#panes [data-id], #tabs [data-id]");
  return target && target.dataset.id !== source ? target : null;
}

function clearDragMarks() {
  document.querySelectorAll(".dragging, .drop-target").forEach((n) => n.classList.remove("dragging", "drop-target"));
  document.body.classList.remove("is-dragging");
}

function makeDraggable(node: HTMLElement, id: string) {
  node.addEventListener("pointerdown", (e) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest("button")) return;
    dragging = { id, x: e.clientX, y: e.clientY, active: false };
    node.setPointerCapture(e.pointerId);
  });
  node.addEventListener("pointermove", (e) => {
    if (!dragging || dragging.id !== id) return;
    if (!dragging.active && Math.hypot(e.clientX - dragging.x, e.clientY - dragging.y) < 6) return;
    dragging.active = true;
    clearDragMarks();
    document.body.classList.add("is-dragging");
    document.querySelectorAll(`[data-id="${id}"]`).forEach((n) => n.classList.add("dragging"));
    const target = dropTarget(e.clientX, e.clientY, id);
    if (target) document.querySelectorAll(`[data-id="${target.dataset.id}"]`).forEach((n) => n.classList.add("drop-target"));
  });
  const finish = (e: PointerEvent, drop: boolean) => {
    if (!dragging || dragging.id !== id) return;
    const wasActive = dragging.active;
    const target = drop && wasActive ? dropTarget(e.clientX, e.clientY, id) : null;
    dragging = null;
    if (!wasActive) return;
    clearDragMarks();
    if (target) action(`acc:swap:${id}:${target.dataset.id}`);
    else render();
  };
  node.addEventListener("pointerup", (e) => finish(e, true));
  node.addEventListener("pointercancel", (e) => finish(e, false));
}

function render() {
  if (!view || dragging?.active) return;
  renderTop(view);
  renderPanes(view);
  renderBottom(view);
}

function renderKick(snapshot: KickSnapshot | null) {
  if (!snapshot) return;
  const players = snapshot.players.map((p) => `${p.playing === true ? "▶" : p.playing === false ? "⏸" : "…"} ${p.key}`).join("  ");
  $("#kick").textContent = `· Kick ${snapshot.channel} (${Math.floor(snapshot.elapsedSecs / 60)} min): ${players}`;
}

const layoutIcons: Record<string, string> = { grid: icons.grid, columns: icons.columns, focus: icons.single };
for (const button of document.querySelectorAll<HTMLButtonElement>(".layouts button")) {
  button.append(iconNode(layoutIcons[button.dataset.layout!]));
  button.addEventListener("click", () => action(`deck:layout:${button.dataset.layout}`));
}
$("#add").prepend(iconNode(icons.plus));
$("#add").addEventListener("click", () => action("deck:add"));
$("#ultra-menu").append(iconNode(icons.chevron));
$("#deck-more").append(iconNode(icons.more));
$("#lives").addEventListener("click", () => action("deck:lives"));
$("#echo").addEventListener("click", () => action("deck:echo"));
$("#ultra").addEventListener("click", () => action("deck:ultra"));
$("#ultra-menu").addEventListener("click", (e) => popupBelow("ultra_menu", e.currentTarget as HTMLElement));
$("#deck-more").addEventListener("click", (e) => popupBelow("deck_menu", e.currentTarget as HTMLElement));
$("#zoom-menu").addEventListener("click", (e) => popupBelow("zoom_menu", e.currentTarget as HTMLElement));
$("#zoom-out").addEventListener("click", () => action("deck:zoom:out"));
$("#zoom-in").addEventListener("click", () => action("deck:zoom:in"));
$("#fullscreen").addEventListener("click", () => action("deck:fullscreen"));
$("#last-buy").addEventListener("click", (e) => {
  const id = (e.currentTarget as HTMLElement).dataset.id;
  if (id) action(`acc:restock:${id}`);
});
addEventListener("keydown", (e) => {
  if (e.key === "F11") {
    e.preventDefault();
    action("deck:fullscreen");
  }
});

listen<DeckView>("deck", (e) => {
  view = e.payload;
  render();
});
listen<{ id: string; report: Report }>("report", (e) => {
  const before = JSON.stringify(reports.get(e.payload.id));
  reports.set(e.payload.id, e.payload.report);
  if (before !== JSON.stringify(e.payload.report)) render();
});
listen<Memory>("memory", (e) => renderMemory(e.payload));
listen<LivesStatus>("lives", (e) => {
  const s = e.payload;
  const changed = JSON.stringify(livesStatus?.accounts) !== JSON.stringify(s.accounts);
  livesStatus = s;
  const pill = $("#lives");
  pill.classList.toggle("on", s.on);
  const label = s.on ? `Lives ${s.playing}/${s.open}` : "Lives";
  const badge = s.redeemable > 0 ? el("span", { className: "pill-badge", textContent: `${s.redeemable} para resgatar` }) : "";
  pill.replaceChildren(el("span", { className: "state" }), label, badge);
  pill.title = s.on
    ? `${s.playing} de ${s.open} lives tocando${s.redeemable ? `; ${s.redeemable} canais com pontos para resgatar` : ""}. Clique para abrir a central das lives.`
    : "Lives da Twitch e da Kick: clique para abrir a central e ligar numa conta";
  const status = $("#lives-status");
  status.textContent = s.text;
  status.title = s.text;
  if (changed) render();
});
$("#lives-status").addEventListener("click", () => action("deck:lives"));
listen<KickSnapshot>("kick", (e) => renderKick(e.payload));
listen<{ id: string; text: string }>("note", (e) => {
  notes.set(e.payload.id, { text: e.payload.text, until: Date.now() + NOTE_MS });
  render();
  setTimeout(render, NOTE_MS + 50);
});

invoke<DeckView | null>("deck_state").then((v) => {
  if (v) view = v;
  render();
});
invoke<KickSnapshot | null>("kick_status").then(renderKick);
setInterval(render, 30_000);
