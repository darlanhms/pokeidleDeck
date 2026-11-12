import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

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
  header: number;
  topBar: number;
  bottomBar: number;
};
type Report = { gold: number | null; nick: string | null; connected: boolean; login: boolean; url: string };
type KickSnapshot = {
  channel: string;
  elapsedSecs: number;
  players: { key: string; playing: boolean | null; checks: number; stoppedChecks: number; detail: string }[];
};

const ZOOM_MIN = 0.5;
const ZOOM_MAX = 1.5;

const $ = <T extends HTMLElement>(selector: string) => document.querySelector<T>(selector)!;
const reports = new Map<string, Report>();
const gold = new Intl.NumberFormat("pt-BR");
let view: DeckView | null = null;

const action = (name: string) => invoke("deck_action", { action: name });
const accountMenu = (id: string) => invoke("account_menu", { id });

function el<K extends keyof HTMLElementTagNameMap>(tag: K, props: Partial<HTMLElementTagNameMap[K]> = {}, ...children: (Node | string)[]) {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

function uptime(since: number): string {
  const minutes = Math.floor((Date.now() - since) / 60000);
  return minutes >= 60 ? `${Math.floor(minutes / 60)} h ${String(minutes % 60).padStart(2, "0")} min` : `${minutes} min`;
}

function info(pane: Pane): { text: string; warn: boolean } {
  if (pane.off) return { text: "desligada", warn: false };
  if (pane.status === "starting") return { text: "abrindo…", warn: false };
  if (pane.status === "loading") return { text: "carregando…", warn: false };
  const report = reports.get(pane.id);
  if (report?.login) return { text: "na tela de login: entre ou desligue a conta", warn: true };
  const parts = [];
  if (pane.startedAt) parts.push(`rodando há ${uptime(pane.startedAt)}`);
  if (report?.connected && report.gold !== null) parts.push(`ouro ${gold.format(report.gold)}`);
  return { text: parts.join(" · "), warn: false };
}

function dotClass(pane: Pane): string {
  if (pane.off) return "";
  if (reports.get(pane.id)?.login) return "login";
  return pane.status === "ok" ? "ok" : pane.status === "loading" ? "loading" : "";
}

function headerButton(text: string, title: string, onClick: () => void, className = "") {
  const button = el("button", { type: "button", textContent: text, title, className });
  button.addEventListener("click", (e) => {
    e.stopPropagation();
    onClick();
  });
  button.addEventListener("dblclick", (e) => e.stopPropagation());
  return button;
}

function renderTop(v: DeckView) {
  $("#top").style.height = `${v.topBar}px`;
  $("#bottom").style.height = `${v.bottomBar}px`;
  for (const button of document.querySelectorAll<HTMLButtonElement>("#layouts button")) {
    button.classList.toggle("active", button.dataset.layout === v.layout);
  }
  $("#account-buttons").replaceChildren(
    ...v.panes.map((pane) => {
      const button = el("button", {
        type: "button",
        textContent: pane.name,
        className: `${pane.focused ? "active" : ""} ${pane.off ? "off" : ""}`,
        title: "Clique: ampliar esta conta (de novo: voltar). Botão direito: mais opções",
      });
      button.addEventListener("click", () => action(`acc:focus:${pane.id}`));
      button.addEventListener("contextmenu", (e) => {
        e.preventDefault();
        accountMenu(pane.id);
      });
      return button;
    }),
  );
  $<HTMLButtonElement>("#zoom-menu").textContent = `Zoom ${Math.round(v.zoom * 100)}%`;
  $<HTMLButtonElement>("#zoom-out").disabled = v.zoom <= ZOOM_MIN + 0.001;
  $<HTMLButtonElement>("#zoom-in").disabled = v.zoom >= ZOOM_MAX - 0.001;
  $<HTMLButtonElement>("#fullscreen").classList.toggle("active", v.fullscreen);
  const on = v.panes.filter((p) => !p.off).length;
  $("#summary").textContent = `${v.panes.length} ${v.panes.length === 1 ? "conta" : "contas"}, ${on} ${on === 1 ? "ligada" : "ligadas"}`;
}

function renderPanes(v: DeckView) {
  const nodes: HTMLElement[] = [];
  for (const pane of v.panes) {
    const r = pane.rect;
    if (!r) continue;
    const status = info(pane);
    const header = el(
      "div",
      { className: "pane-header" },
      el("span", { className: `dot ${dotClass(pane)}`, textContent: "●" }),
      el("span", { className: "name", textContent: pane.name }),
      el("span", { className: `info ${status.warn ? "warn" : ""}`, textContent: status.text, title: status.text }),
    );
    if (!pane.off) {
      header.append(
        headerButton(pane.muted ? "🔇" : "🔊", "Ligar / desligar o som desta conta", () => action(`acc:mute:${pane.id}`)),
        headerButton("⟳", "Recarregar a página", () => action(`acc:reload:${pane.id}`)),
      );
    }
    header.append(
      headerButton(pane.focused ? "⤡" : "⤢", "Ampliar esta conta / voltar (duplo clique no cabeçalho faz o mesmo)", () =>
        action(`acc:focus:${pane.id}`),
      ),
      headerButton(
        "⏻",
        pane.off
          ? "Ligar esta conta: o jogo abre de novo"
          : "Desligar esta conta: o jogo dela para (ela continua aqui no deck)",
        () => action(`acc:power:${pane.id}`),
        `power ${pane.off ? "off" : ""}`,
      ),
    );
    Object.assign(header.style, { left: `${r.x}px`, top: `${r.y}px`, width: `${r.w}px`, height: `${v.header}px` });
    header.addEventListener("dblclick", () => action(`acc:focus:${pane.id}`));
    header.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      accountMenu(pane.id);
    });
    nodes.push(header);

    if (pane.off) {
      const note = el(
        "div",
        { className: "off-note" },
        el("strong", { textContent: `${pane.name} desligada` }),
        el("span", { textContent: "Nada roda nela: o jogo fica parado." }),
        el("span", { textContent: "Clique no ⏻ do cabeçalho para ligar." }),
      );
      note.addEventListener("dblclick", () => action(`acc:focus:${pane.id}`));
      Object.assign(note.style, {
        left: `${r.x}px`,
        top: `${r.y + v.header}px`,
        width: `${r.w}px`,
        height: `${Math.max(0, r.h - v.header)}px`,
      });
      nodes.push(note);
    }
  }
  $("#panes").replaceChildren(...nodes);
}

function render() {
  if (!view) return;
  renderTop(view);
  renderPanes(view);
}

function renderKick(snapshot: KickSnapshot | null) {
  if (!snapshot) return;
  const players = snapshot.players
    .map((p) => `${p.playing === true ? "▶" : p.playing === false ? "⏸" : "…"} ${p.key}`)
    .join("  ");
  $("#kick").textContent = `Kick ${snapshot.channel} (${Math.floor(snapshot.elapsedSecs / 60)} min): ${players}`;
}

for (const button of document.querySelectorAll<HTMLButtonElement>("#layouts button")) {
  button.addEventListener("click", () => action(`deck:layout:${button.dataset.layout}`));
}
$("#accounts-menu").addEventListener("click", (e) => {
  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
  invoke("deck_menu", { x: rect.left, y: rect.bottom });
});
$("#zoom-menu").addEventListener("click", (e) => {
  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
  invoke("zoom_menu", { x: rect.left, y: rect.bottom });
});
$("#zoom-out").addEventListener("click", () => action("deck:zoom:out"));
$("#zoom-in").addEventListener("click", () => action("deck:zoom:in"));
$("#fullscreen").addEventListener("click", () => action("deck:fullscreen"));
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
listen<KickSnapshot>("kick", (e) => renderKick(e.payload));

invoke<DeckView | null>("deck_state").then((v) => {
  if (v) view = v;
  render();
});
invoke<KickSnapshot | null>("kick_status").then(renderKick);
setInterval(() => view && renderPanes(view), 30_000);
