import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type Pane = { id: string; name: string; x: number; y: number; w: number; h: number };
type Layout = { panes: Pane[]; header: number; topBar: number; bottomBar: number };
type Report = { gold: number | null; nick: string | null; connected: boolean; login: boolean; url: string };
type Player = {
  key: string;
  playing: boolean | null;
  currentTime: number;
  checks: number;
  stoppedChecks: number;
  resumes: number;
  detail: string;
};
type KickSnapshot = { channel: string; elapsedSecs: number; players: Player[] };

const $ = <T extends HTMLElement>(selector: string) => document.querySelector<T>(selector)!;
const panesEl = $("#panes");
const message = $("#message");
const reports = new Map<string, Report>();
let layout: Layout | null = null;

const gold = new Intl.NumberFormat("pt-BR");

function headerInfo(report: Report | undefined): { text: string; css: string } {
  if (!report) return { text: "abrindo…", css: "" };
  if (report.login) return { text: "na tela de login", css: "login" };
  if (report.connected && report.gold !== null)
    return { text: `${report.nick ?? ""} · ouro ${gold.format(report.gold)}`, css: "ok" };
  return { text: new URL(report.url).host, css: "" };
}

function renderPanes() {
  if (!layout) return;
  const { header, topBar, bottomBar } = layout;
  $("#top").style.height = `${topBar}px`;
  $("#bottom").style.height = `${bottomBar}px`;
  panesEl.replaceChildren(
    ...layout.panes.map((pane) => {
      const el = document.createElement("div");
      el.className = "pane-header";
      Object.assign(el.style, {
        left: `${pane.x}px`,
        top: `${pane.y}px`,
        width: `${pane.w}px`,
        height: `${header}px`,
      });
      const info = headerInfo(reports.get(pane.id));
      el.innerHTML = `<span class="dot ${info.css}">●</span><strong></strong><span class="info"></span><button class="more" type="button" title="Opções da conta">⋯</button>`;
      el.querySelector("strong")!.textContent = pane.name;
      el.querySelector(".info")!.textContent = info.text;
      const menu = (e: Event) => {
        e.preventDefault();
        invoke("account_menu", { id: pane.id });
      };
      el.addEventListener("contextmenu", menu);
      el.querySelector(".more")!.addEventListener("click", menu);
      return el;
    }),
  );
}

function renderKick(snapshot: KickSnapshot | null) {
  const el = $("#kick");
  if (!snapshot) return;
  const minutes = Math.floor(snapshot.elapsedSecs / 60);
  el.replaceChildren(`Kick ${snapshot.channel} (${minutes} min): `);
  for (const p of snapshot.players) {
    const span = document.createElement("span");
    span.className = `player ${p.playing === true ? "playing" : p.playing === false ? "stopped" : ""}`;
    const symbol = p.playing === true ? "▶" : p.playing === false ? "⏸" : "…";
    span.textContent = ` ${symbol} ${p.key} (${p.checks - p.stoppedChecks}/${p.checks}) `;
    span.title = p.detail;
    el.append(span);
  }
}

$("#accounts").addEventListener("click", (e) => {
  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
  invoke("deck_menu", { x: rect.left, y: rect.bottom });
});

const PROMISE_PROBE = `return await new Promise((resolve) => setTimeout(() => resolve({
  title: document.title,
  gold: window.__deckRecompra?.summary()?.gold ?? null,
  waitedMs: 750,
}), 750));`;

$("#promise").addEventListener("click", async () => {
  if (!layout) return;
  message.textContent = "esperando as Promises…";
  const results = await Promise.all(
    layout.panes.map(async (pane) => {
      const started = performance.now();
      try {
        const json = await invoke<string>("account_eval", { id: pane.id, script: PROMISE_PROBE });
        return `${pane.name}: ${json} em ${Math.round(performance.now() - started)} ms`;
      } catch (err) {
        return `${pane.name}: erro ${err}`;
      }
    }),
  );
  message.textContent = results.join(" | ");
  console.log(results);
});

listen<Layout>("layout", (e) => {
  layout = e.payload;
  renderPanes();
});
listen<{ id: string; report: Report }>("report", (e) => {
  const before = JSON.stringify(reports.get(e.payload.id));
  reports.set(e.payload.id, e.payload.report);
  if (before !== JSON.stringify(e.payload.report)) renderPanes();
});
listen<KickSnapshot>("kick", (e) => renderKick(e.payload));

invoke<Layout | null>("deck_state").then((l) => {
  layout = l;
  renderPanes();
});
invoke<KickSnapshot | null>("kick_status").then(renderKick);
