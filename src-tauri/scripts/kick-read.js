// Saldo e os 3 bônus do PokeIdle de um canal da Kick, rodando num player da Kick da conta. Copiado do
// original (BulkRedeemDialog, leitura da Kick).
(async a => {
 const head = { Accept: 'application/json', 'x-app-platform': 'web', Authorization: 'Bearer ' + a.token };
 const ch = a.api + '/api/v2/channels/' + encodeURIComponent(a.channel);
 const get = async url => { try { const r = await fetch(url, { cache: 'no-cache', credentials: 'include', headers: head });
  return { status: r.status, json: r.ok ? await r.json().catch(() => null) : null }; }
  catch (e) { return { status: 0, json: null, fail: (e && e.message) || String(e) }; } };
 // O canal vende o mesmo bônus em pacotes (1h, 3h, 5h, 10h), todos pelo mesmo preço por hora: fica o maior
 // que o saldo paga; sem saldo para nenhum, o mais barato (o que falta menos).
 const pick = (all, points) => a.words.map(w => { const word = new RegExp(w, 'i');
  const list = all.filter(r => word.test(r.title) && /pokeidle/i.test(r.title + ' ' + (r.prompt || r.about || '')));
  const open = list.filter(r => !r.blocked), pool = open.length ? open : list;
  const fits = typeof points === 'number' ? pool.filter(r => r.cost <= points).sort((x, y) => y.cost - x.cost) : [];
  return fits[0] ?? pool.sort((x, y) => x.cost - y.cost)[0] ?? null; });
 try {
  const [me, info, mine, pts, rws] = await Promise.all([get(a.api + '/api/v1/user'), get(ch), get(ch + '/me'),
   get(ch + '/points'), get(ch + '/rewards')]);
  if (rws.fail) return { error: 'sem resposta da Kick: ' + rws.fail };
  if (info.status === 404 || rws.status === 404) return { error: 'canal não encontrado na Kick' };
  const user = me.json;
  const owner = mine.json?.is_broadcaster === true
   || (user?.id != null && (user.id === info.json?.user_id || user.id === info.json?.user?.id))
   || (!!user?.streamer_channel?.slug && user.streamer_channel.slug === a.channel);
  const points = pts.json?.data?.points;
  if (!owner && typeof points !== 'number') return { error: 'sem o saldo na resposta (erro ' + pts.status + '; a conta entrou na Kick?)' };
  if (!rws.json) return { error: 'os bônus do canal: erro ' + rws.status };
  const all = (rws.json.data || []).filter(r => r.is_enabled).map(r => ({ id: String(r.id), title: r.title || '', prompt: null,
   about: r.description || '', cost: r.cost, blocked: r.is_paused ? 'pausado no canal' : null }));
  return { channelId: a.channel, points: typeof points === 'number' ? points : null, owner, login: user?.username ?? null, rewards: pick(all, points) };
 } catch (e) { return { error: 'sem resposta da Kick: ' + (e && e.message || e) }; }
})(__ARGS__).then(o => JSON.stringify(o))
