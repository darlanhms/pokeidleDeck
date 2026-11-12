// Eco: cada clique de verdade nesta conta vira uma mensagem para o deck, que repete o mesmo botão nas
// outras (pelo caminho e pelo texto dele, não pela posição na tela). Os cliques que o deck repete não são
// "de verdade" (isTrusted falso), então não voltam em eco. O que se digita nunca é repetido.
(() => {
  if (window.top !== window) return;
  const textOf = (el) => String(el.getAttribute('aria-label') || el.getAttribute('title') || el.innerText || el.value || '')
    .trim().replace(/\s+/g, ' ').slice(0, 80);
  const clickable = 'button, a, input, select, textarea, label, summary, [role=button], [role=tab], [role=menuitem], [role=option], [onclick], [tabindex]';
  const pathOf = (el) => {
    const parts = [];
    for (; el && el !== document.body && el.parentElement; el = el.parentElement) {
      const same = [...el.parentElement.children].filter((c) => c.tagName === el.tagName);
      parts.unshift(el.tagName.toLowerCase() + (same.length > 1 ? `:nth-of-type(${same.indexOf(el) + 1})` : ''));
    }
    return parts.length ? 'body > ' + parts.join(' > ') : 'body';
  };
  addEventListener('click', (e) => {
    if (!e.isTrusted || !(e.target instanceof Element)) return;
    const invoke = window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke;
    if (typeof invoke !== 'function') return;
    const el = e.target.closest(clickable) || e.target;
    const r = el.getBoundingClientRect();
    const click = {
      path: pathOf(el), tag: el.tagName.toLowerCase(), text: textOf(el),
      fx: r.width ? (e.clientX - r.left) / r.width : 0.5, fy: r.height ? (e.clientY - r.top) / r.height : 0.5,
    };
    invoke('account_click', { click }).catch(() => { });
  }, true);
})();
