// O player da Kick pausa (e nem carrega o stream) quando a página fica oculta. Fora da tela ela está
// sempre oculta, então a página passa a se ver sempre visível. Quando o WebKit para o
// requestAnimationFrame da página oculta, ele segue por setTimeout.
(() => {
  const proto = Document.prototype;
  const realState = Object.getOwnPropertyDescriptor(proto, 'visibilityState').get;
  const realHidden = () => realState.call(document) !== 'visible';
  Object.defineProperty(window, '__deckRealVisibility', { value: () => realState.call(document) });
  Object.defineProperty(proto, 'visibilityState', { get: () => 'visible', configurable: true });
  Object.defineProperty(proto, 'hidden', { get: () => false, configurable: true });
  Object.defineProperty(proto, 'webkitVisibilityState', { get: () => 'visible', configurable: true });
  Object.defineProperty(proto, 'webkitHidden', { get: () => false, configurable: true });
  proto.hasFocus = () => true;
  for (const type of ['visibilitychange', 'webkitvisibilitychange', 'pagehide', 'blur']) {
    window.addEventListener(type, (e) => { if (e.target === window || e.target === document) e.stopImmediatePropagation(); }, true);
    document.addEventListener(type, (e) => e.stopImmediatePropagation(), true);
  }
  const raf = window.requestAnimationFrame.bind(window);
  const caf = window.cancelAnimationFrame.bind(window);
  window.requestAnimationFrame = (cb) => realHidden() ? setTimeout(() => cb(performance.now()), 16) : raf(cb);
  window.cancelAnimationFrame = (id) => { clearTimeout(id); caf(id); };
})();
