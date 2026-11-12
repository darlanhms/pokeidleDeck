import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type Reward = "xp" | "capture" | "lure";
type SlotChoice = { kind: "auto" } | { kind: "off" } | { kind: "channel"; channel: string };
type Tab = { id: string; name: string; livesOn: boolean; off: boolean; playing: number; redeemable: number; warn: boolean };
type RewardView = { kind: Reward; title: string; cost: number; blocked: string | null; affordable: boolean };
type Row = {
  channel: string;
  live: boolean | null;
  state: "playing" | "standin" | "queued" | "offline" | "closed" | "unknown";
  slot: number | null;
  standInFor: string | null;
  playing: boolean | null;
  points: number | null;
  owner: boolean;
  ready: boolean;
  rewards: RewardView[];
  error: string | null;
};
type SlotPicker = { index: number; choice: SlotChoice; current: string | null; warn: boolean };
type SiteView = {
  site: "Twitch" | "Kick";
  name: string;
  login: boolean | null;
  autoRedeem: Reward | null;
  slots: SlotPicker[];
  notes: string[];
  rows: Row[];
};
type AccountView = {
  id: string;
  name: string;
  livesOn: boolean;
  off: boolean;
  playing: number;
  slots: number;
  redeemable: number;
  chest: string | null;
  claimChest: boolean;
  sites: SiteView[];
  feed: { text: string; meta: string }[];
};
type View = { tabs: Tab[]; account: AccountView | null };

const REWARD_LABEL: Record<Reward, string> = { xp: "+15% XP", capture: "+15% Captura", lure: "+15% Shiny" };
const fmt = (n: number) => n.toLocaleString("pt-BR");
const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

let view: View | null = null;
const FOCUS_KEY = "central.ocultar";
let focusOnly = (() => {
  try {
    return localStorage.getItem(FOCUS_KEY) === "1";
  } catch {
    return false;
  }
})();

/** Some com o interruptor ligado: fora das vagas, sem resgate pronto, e offline ou sem o bônus. */
function hidden(row: Row): boolean {
  if (!focusOnly || row.slot !== null || row.ready) return false;
  const noBonus = row.points !== null && row.rewards.length === 0;
  return row.state === "offline" || row.state === "closed" || noBonus;
}
let openMenu: string | null = null;
const rowMessages = new Map<string, { text: string; ok: boolean }>();

function el<K extends keyof HTMLElementTagNameMap>(tag: K, props: Record<string, unknown> = {}, ...children: (Node | string | null)[]): HTMLElementTagNameMap[K] {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children.filter((c): c is Node | string => c !== null));
  return node;
}

const action = (name: string, extra: Record<string, unknown> = {}) => invoke("central_action", { action: name, ...extra });

function renderTabs(v: View) {
  $("tabs").replaceChildren(
    ...v.tabs.map((tab) => {
      const current = v.account?.id === tab.id;
      const button = el(
        "button",
        { type: "button", className: current ? "current" : "" },
        el("span", { className: `dot ${tab.off || !tab.livesOn ? "" : "ok"}` }),
        el("strong", { textContent: tab.name }),
        !tab.livesOn ? el("span", { className: "muted", textContent: "sem lives" }) : null,
        tab.redeemable > 0 ? el("span", { className: `badge ${current ? "" : "soft"}`, textContent: String(tab.redeemable), title: `${tab.redeemable} canais com pontos para resgatar` }) : null,
      );
      if (current) button.ariaCurrent = "page";
      button.addEventListener("click", () => invoke("central_select", { id: tab.id }));
      return button;
    }),
  );
}

function card(label: string, value: Node | string, accent = false) {
  return el("div", { className: `card ${accent ? "accent" : ""}` }, el("span", { className: "label", textContent: label }), el("strong", {}, value));
}

function nextText(row: Row, site: SiteView): { text: string; ready: boolean } {
  if (row.points === null && !row.owner) {
    if (site.login === false) return { text: `entre na ${site.name} para ver o saldo`, ready: false };
    return { text: row.error ?? "lendo o saldo…", ready: false };
  }
  if (!row.rewards.length) return { text: "canal sem o bônus do PokeIdle", ready: false };
  if (row.ready) return { text: "pronto para resgatar", ready: true };
  const cheapest = [...row.rewards].filter((r) => !r.blocked).sort((a, b) => a.cost - b.cost)[0];
  if (!cheapest) return { text: "bônus pausado no canal", ready: false };
  return { text: `faltam ${fmt(cheapest.cost - (row.points ?? 0))} para o ${REWARD_LABEL[cheapest.kind].replace("+15% ", "")}`, ready: false };
}

function stateText(row: Row, site: SiteView): string {
  switch (row.state) {
    case "playing": {
      const chosen = site.slots[row.slot!]?.choice.kind === "channel";
      return `▶ vaga ${row.slot! + 1}${chosen ? " · escolhido" : ""}${row.playing === false ? " · travado" : ""}`;
    }
    case "standin":
      return `▶ vaga ${row.slot! + 1} · no lugar de ${row.standInFor}`;
    case "queued":
      return site.login === false || !site.slots.length ? "no ar" : "no ar · vagas cheias";
    case "offline":
      return "offline";
    case "closed":
      return "fora da lista desta conta";
    default:
      return "conferindo…";
  }
}

function menuFor(account: AccountView, site: SiteView, row: Row): HTMLElement {
  const menu = el("div", { className: "menu", role: "menu" });
  const close = () => {
    openMenu = null;
    render();
  };
  if (row.rewards.length) {
    menu.append(el("span", { className: "menu-title", textContent: "Resgatar" }));
    for (const reward of row.rewards) {
      const missing = (reward.cost - (row.points ?? 0));
      const detail = reward.blocked ?? (reward.affordable ? fmt(reward.cost) : `faltam ${fmt(missing)}`);
      const item = el("button", { type: "button", role: "menuitem", disabled: !reward.affordable }, REWARD_LABEL[reward.kind], el("span", { textContent: detail }));
      item.addEventListener("click", async () => {
        close();
        rowMessages.set(`${site.site}:${row.channel}`, { text: "resgatando…", ok: true });
        render();
        try {
          const text = await invoke<string>("central_redeem", { id: account.id, site: site.site, channel: row.channel, kind: reward.kind });
          rowMessages.set(`${site.site}:${row.channel}`, { text, ok: true });
        } catch (err) {
          rowMessages.set(`${site.site}:${row.channel}`, { text: String(err), ok: false });
        }
        render();
      });
      menu.append(item);
    }
  }
  if (site.slots.length && row.state !== "closed") {
    menu.append(el("span", { className: "menu-title", textContent: `Tocar ${row.channel} na` }));
    for (const slot of site.slots) {
      const here = slot.current === row.channel;
      const item = el(
        "button",
        { type: "button", role: "menuitem", disabled: here && slot.choice.kind === "channel" },
        `Vaga ${slot.index + 1}`,
        el("span", { textContent: here ? "já está nela" : slot.current ? `no lugar de ${slot.current}` : "vazia" }),
      );
      item.addEventListener("click", () => {
        close();
        invoke("central_choose", { id: account.id, site: site.site, slot: slot.index, choice: { kind: "channel", channel: row.channel } });
      });
      menu.append(item);
    }
    menu.append(el("span", { className: "menu-hint", textContent: "Se sair do ar, a vaga toca a 1ª da lista no ar e volta para ele quando voltar." }));
  }
  const toggle = el("button", { type: "button", role: "menuitem" }, row.state === "closed" ? "Voltar para a lista desta conta" : "Tirar da lista desta conta");
  toggle.addEventListener("click", () => {
    close();
    invoke("central_channel", { id: account.id, site: site.site, channel: row.channel, open: row.state === "closed" });
  });
  menu.append(el("div", { className: "menu-sep" }), toggle);
  return menu;
}

function renderRow(account: AccountView, site: SiteView, row: Row): HTMLElement {
  const key = `${site.site}:${row.channel}`;
  const next = nextText(row, site);
  const cheapest = Math.min(...row.rewards.filter((r) => !r.blocked).map((r) => r.cost), 300);
  const progress = row.ready ? 100 : row.points === null ? 0 : Math.min(100, (row.points / cheapest) * 100);
  const canPlay = row.state === "queued" && site.slots.length > 0 && site.login !== false;
  const primaryLabel = row.ready ? "Resgatar ▾" : canPlay ? "Tocar na vaga ▾" : row.state === "closed" ? "Voltar à lista" : "⋯";
  const primary = el("button", {
    type: "button",
    className: `row-action ${row.ready ? "ready" : primaryLabel === "⋯" ? "quiet" : ""}`,
    textContent: primaryLabel,
    title: primaryLabel === "⋯" ? "Resgatar, tocar numa vaga ou tirar da lista" : "",
  });
  primary.setAttribute("aria-haspopup", "menu");
  primary.addEventListener("click", (e) => {
    e.stopPropagation();
    if (row.state === "closed" && !row.ready) {
      invoke("central_channel", { id: account.id, site: site.site, channel: row.channel, open: true });
      return;
    }
    openMenu = openMenu === key ? null : key;
    render();
  });
  const message = rowMessages.get(key);
  const fill = el("span", { className: `fill ${row.ready ? "ready" : ""}` });
  fill.style.width = `${progress}%`;
  const live = row.live === true ? "ok" : row.live === false ? "" : "unknown";
  return el(
    "div",
    { className: `row ${openMenu === key ? "open" : ""}`, role: "row" },
    el("span", { className: "cell channel", role: "cell" }, el("span", { className: `dot ${live}` }), el("strong", { textContent: row.channel })),
    el("span", { className: `cell state ${row.state}`, role: "cell", textContent: stateText(row, site) }),
    el(
      "span",
      { className: "cell points", role: "cell" },
      el(
        "span",
        {},
        el("strong", { textContent: row.owner ? "∞" : row.points === null ? "—" : fmt(row.points) }),
        " ",
        el("span", { className: `next ${next.ready ? "ready" : ""}`, textContent: message ? message.text : next.text }),
      ),
      el("span", { className: "bar" }, fill),
    ),
    el("span", { className: "cell row-actions", role: "cell" }, primary, openMenu === key ? menuFor(account, site, row) : null),
  );
}

function slotSelect(account: AccountView, site: SiteView, slot: SlotPicker): HTMLElement {
  const select = el("select", { ariaLabel: `Vaga ${slot.index + 1} da ${site.name}` });
  const value = slot.choice.kind === "channel" ? `c:${slot.choice.channel}` : slot.choice.kind;
  const autoText = slot.choice.kind === "auto" && slot.current ? `Automático (${slot.current})` : "Automático";
  select.append(new Option(autoText, "auto"));
  for (const row of site.rows) {
    const suffix = row.live === false ? " · offline" : "";
    select.append(new Option(`${row.channel}${suffix}`, `c:${row.channel}`));
  }
  select.append(new Option("Nenhuma", "off"));
  select.value = value;
  select.classList.toggle("warn", slot.warn);
  select.addEventListener("change", () => {
    const v = select.value;
    const choice: SlotChoice = v === "auto" ? { kind: "auto" } : v === "off" ? { kind: "off" } : { kind: "channel", channel: v.slice(2) };
    invoke("central_choose", { id: account.id, site: site.site, slot: slot.index, choice });
  });
  return el("label", { className: "slot" }, `Vaga ${slot.index + 1}`, select);
}

function renderSite(account: AccountView, site: SiteView): HTMLElement | null {
  if (!site.rows.length) return null;
  const auto = site.autoRedeem ? `resgate automático: ${REWARD_LABEL[site.autoRedeem]}` : "resgate automático desligado";
  return el(
    "section",
    { className: "site", ariaLabel: site.name },
    el(
      "div",
      { className: "site-head" },
      el("strong", { textContent: site.name }),
      el("span", { className: "muted", textContent: site.slots.length ? auto : "nenhuma vaga (Canais e config…)" }),
      el("span", { className: "spacer" }),
      ...site.slots.map((slot) => slotSelect(account, site, slot)),
    ),
    ...site.notes.map((note) => el("div", { className: "site-note" }, el("span", { className: "dot loading" }), note)),
    el(
      "div",
      { className: "table", role: "table" },
      el(
        "div",
        { className: "row head", role: "row" },
        el("span", { role: "columnheader", textContent: "Canal" }),
        el("span", { role: "columnheader", textContent: "Agora" }),
        el("span", { role: "columnheader", textContent: "Pontos" }),
        el("span", { role: "columnheader" }),
      ),
      ...site.rows.filter((row) => !hidden(row)).map((row) => renderRow(account, site, row)),
    ),
    ...(() => {
      const count = site.rows.filter(hidden).length;
      if (!count) return [];
      const show = el("button", { type: "button", className: "link", textContent: "mostrar" });
      show.addEventListener("click", () => setFocusOnly(false));
      return [el("div", { className: "hidden-note" }, `${count} ${count === 1 ? "canal oculto" : "canais ocultos"} (offline ou sem o bônus) · `, show)];
    })(),
  );
}

function renderMain(v: View) {
  const main = $("main");
  const account = v.account;
  if (!account) {
    main.replaceChildren(el("p", { className: "empty", textContent: "Nenhuma conta no deck." }));
    return;
  }
  if (account.off || !account.livesOn) {
    const button = el("button", { type: "button", className: "primary", textContent: account.off ? "Ligar a conta" : "Ligar as lives nesta conta" });
    button.addEventListener("click", () => action(account.off ? "turn-on" : "toggle-lives", { id: account.id }));
    main.replaceChildren(
      el(
        "div",
        { className: "empty" },
        el("strong", { textContent: account.off ? `${account.name} está desligada` : `Lives desligadas em ${account.name}` }),
        el("span", { textContent: "Marque as lives só na conta vinculada à sua Twitch/Kick (normalmente a main): o bônus vale para a conta do jogo ligada a elas." }),
        button,
      ),
    );
    return;
  }
  const autos = account.sites.filter((s) => s.autoRedeem).map((s) => `${REWARD_LABEL[s.autoRedeem!].replace("+15% ", "")} na ${s.name}`);
  const cards = el(
    "div",
    { className: "cards" },
    card("Tocando nesta conta", `${account.playing} de ${account.slots} vagas`),
    card("Prontos para resgatar", `${account.redeemable} ${account.redeemable === 1 ? "canal" : "canais"}`, account.redeemable > 0),
    card("Baú da Twitch", !account.claimChest ? "desligado" : account.chest ? `pego às ${account.chest}` : "esperando o primeiro"),
    card("Resgate automático", autos.length ? autos.join(", ") : "desligado"),
  );
  const banners = account.sites
    .filter((s) => s.slots.length && s.login === false)
    .map((s) => {
      const button = el("button", { type: "button", textContent: `Entrar na ${s.name}` });
      button.addEventListener("click", () => action("login", { id: account.id, site: s.site }));
      const text = s.site === "Twitch" ? "Twitch sem login nesta conta: os players da Twitch esperam o login." : "Kick sem login nesta conta: os players tocam, mas não juntam pontos.";
      return el("div", { className: "banner warn", role: "status" }, el("span", { textContent: text }), button);
    });
  const sites = account.sites.map((s) => renderSite(account, s)).filter((s): s is HTMLElement => s !== null);
  const help = el(
    "div",
    { className: "help" },
    el("strong", { textContent: "Cada vaga toca o canal que você escolher" }),
    el("span", { textContent: "Escolha o canal da vaga, ou deixe em Automático (a 1ª da lista no ar). Se ele sair do ar, a vaga toca a 1ª da lista que estiver no ar e livre, e volta sozinha quando ele voltar." }),
  );
  main.replaceChildren(cards, ...banners, ...sites, help);
}

function renderFeed(v: View) {
  const feed = $("feed");
  const link = el("button", { type: "button", className: "link", textContent: "Registro completo" });
  link.addEventListener("click", () => action("log"));
  const items = v.account?.feed ?? [];
  feed.replaceChildren(
    el("div", { className: "feed-head" }, el("strong", { textContent: "Atividade" }), link),
    items.length
      ? el("ol", {}, ...items.map((i) => el("li", {}, el("span", { textContent: i.text }), el("span", { className: "meta", textContent: i.meta }))))
      : el("p", { className: "muted", textContent: "Os resgates, os baús e as recusas aparecem aqui." }),
  );
}

function render() {
  if (!view) return;
  const focused = document.activeElement?.tagName === "SELECT";
  renderTabs(view);
  if (!focused) renderMain(view);
  renderFeed(view);
  ($("players") as HTMLButtonElement).disabled = !view.account?.livesOn;
}

function setFocusOnly(on: boolean) {
  focusOnly = on;
  try {
    localStorage.setItem(FOCUS_KEY, on ? "1" : "0");
  } catch {
    // Sem armazenamento: vale só até fechar a janela.
  }
  $("focus-only").setAttribute("aria-checked", String(on));
  render();
}
$("focus-only").setAttribute("aria-checked", String(focusOnly));
$("focus-only").addEventListener("click", () => setFocusOnly(!focusOnly));

document.addEventListener("click", () => {
  if (openMenu) {
    openMenu = null;
    render();
  }
});
$("players").addEventListener("click", () => view?.account && action("players", { id: view.account.id }));
$("config").addEventListener("click", () => action("config"));
listen<View>("central", (e) => {
  view = e.payload;
  render();
});
invoke<View>("central_state").then((v) => {
  view = v;
  render();
});
