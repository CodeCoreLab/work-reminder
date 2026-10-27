// A release build has no console and, once the window is gone, no UI to draw into.
// Send errors to the backend, which appends them to work-reminder.log.
function reportError(where, detail) {
  const msg = `${where}: ${detail}`;
  document.title = '出错：' + msg;
  try {
    window.__TAURI__.core.invoke('ui_log', { message: msg });
  } catch (_) {
    // The bridge itself is what failed; the title is all that is left.
  }
}

window.addEventListener('error', (e) => reportError('error', e.message || '未知'));
window.addEventListener('unhandledrejection', (e) => {
  const r = e.reason;
  reportError('rejection', r && r.message ? r.message : String(r));
});

// Copyright (c) 2026 Ma Xin <maxin@example.com>
// SPDX-License-Identifier: MIT
//
// Presentation only: no timer, no storage, no scheduling. Every due time comes from
// the backend snapshot, so there is exactly one implementation of the twelve
// schedule types (in Rust) instead of one per front end.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);

const el = {
  autostart: $('autostart'),
  empty: $('empty'),
  table: $('table'),
  rows: $('rows'),
  new: $('new'),
  edit: $('edit'),
  toggle: $('toggle'),
  delete: $('delete'),
  about: $('about'),
  close: $('close'),
  sheet: $('sheet'),
  sheetTitle: $('sheetTitle'),
  form: $('form'),
  title: $('f-title'),
  message: $('f-message'),
  kind: $('f-kind'),
  params: $('params'),
  enabled: $('f-enabled'),
  formError: $('formError'),
  cancel: $('cancel'),
  stack: $('stack'),
};

/// Full snapshot from the backend.
let state = { tasks: [], now: '', loadError: null };
/// Currently selected task id, or null.
let selected = null;
/// Kind -> [(label, hint)] so the form can be rebuilt without a round trip per key.
let paramSpecs = new Map();
/// True while the sheet is editing an existing task.
let editingId = '';
/// How many reminder cards may stack at once.
const MAX_TOASTS = 4;

/// Run a mutating command, adopt the returned snapshot, and surface a failure.
///
/// Every `await invoke(...)` used to be bare: a rejected promise skipped `render()`,
/// so the row kept showing the old state and nothing told the user why. Now a
/// failure produces a visible message instead of a silently wrong list.
async function mutate(command, args, toastTitle) {
  try {
    const next = await invoke(command, args);
    if (next && Array.isArray(next.tasks)) {
      state = next;
      render();
    }
    return true;
  } catch (err) {
    showToast({
      id: '',
      title: toastTitle || '操作失败',
      message: String(err),
      usageSeconds: 0,
      preAlert: false,
    });
    return false;
  }
}

// ------------------------------------------------------------------ rendering

function render() {
  const tasks = state.tasks;
  const isEmpty = tasks.length === 0;

  el.empty.hidden = !isEmpty;
  el.table.hidden = isEmpty;

  el.rows.replaceChildren(...tasks.map(rowOf));

  // Selection can be stale after a delete or a reload.
  if (selected && !tasks.some((t) => t.id === selected)) {
    selected = null;
  }
  syncActions();
  markSelected();
}

function rowOf(task) {
  const row = document.createElement('div');
  row.className = 'row';
  row.dataset.id = task.id;
  if (!task.enabled) row.classList.add('is-paused');
  if (task.overdue) row.classList.add('is-overdue');

  // Column 1: the enable checkbox.
  const checkWrap = document.createElement('div');
  checkWrap.className = 'check-wrap';
  const box = document.createElement('button');
  box.type = 'button';
  box.className = 'check-box';
  box.setAttribute('role', 'checkbox');
  box.setAttribute('aria-checked', String(task.enabled));
  box.setAttribute('aria-label', `${task.title}：${task.enabled ? '已启用' : '已暂停'}`);
  box.innerHTML =
    '<svg viewBox="0 0 24 24"><path d="M4 12.5 9 17.5 20 6.5"/></svg>';
  box.addEventListener('click', async (e) => {
    e.stopPropagation();
    await mutate('set_enabled', { id: task.id, enabled: !task.enabled }, '切换启用状态失败');
  });
  checkWrap.append(box);

  const title = document.createElement('div');
  title.className = 'cell cell--title';
  title.textContent = task.title;
  title.title = task.message ? `${task.title}\n${task.message}` : task.title;

  const sched = document.createElement('div');
  sched.className = 'cell cell--sched';
  sched.textContent = task.scheduleText;

  const next = document.createElement('div');
  next.className = 'cell cell--next';
  next.textContent = task.nextText;

  row.append(checkWrap, title, sched, next);

  row.addEventListener('click', () => {
    selected = task.id;
    markSelected();
    syncActions();
  });
  row.addEventListener('dblclick', () => openSheet(task));

  return row;
}

function markSelected() {
  for (const row of el.rows.children) {
    row.classList.toggle('is-selected', row.dataset.id === selected);
  }
}

function syncActions() {
  const has = Boolean(selected);
  el.edit.disabled = !has;
  el.toggle.disabled = !has;
  el.delete.disabled = !has;
  const task = state.tasks.find((t) => t.id === selected);
  el.toggle.textContent = task && task.enabled ? '暂停' : '启用';
}

// --------------------------------------------------------------------- sheet

function openSheet(task) {
  editingId = task ? task.id : '';
  el.sheetTitle.textContent = task ? '编辑提醒任务' : '设置提醒任务';
  el.formError.hidden = true;

  el.title.value = task ? task.title : '';
  el.message.value = task ? task.message : '';
  el.kind.value = task ? task.kind : el.kind.options[0]?.value ?? '';
  el.enabled.checked = task ? task.enabled : true;

  rebuildParams(task ? task.schedule : null);
  el.sheet.showModal();
  // Focus the first field: opening a dialog should land the caret where typing goes.
  requestAnimationFrame(() => el.title.focus());
}

/// Render the parameter inputs for the current kind.
///
/// `schedule` prefills the fields when editing; without it the fields start empty
/// and the placeholder explains the expected format.
function rebuildParams(schedule) {
  const kind = el.kind.value;
  const spec = paramSpecs.get(kind) ?? [];
  // Prefill only when the stored schedule is still the selected kind. After a kind
  // change the old values are meaningless in the new fields, so defaults are used.
  const sameKind = schedule && schedule.scheduleType === kind;
  const prefill = sameKind ? (prefillFor(schedule) ?? []) : [];

  el.params.replaceChildren();
  spec.forEach(([label, hint, preset], i) => {
    const wrap = document.createElement('div');
    wrap.className = 'field';

    const id = `p-${i}`;
    const lab = document.createElement('label');
    lab.htmlFor = id;
    lab.textContent = label;

    const input = document.createElement('input');
    input.id = id;
    input.type = 'text';
    input.autocomplete = 'off';
    input.placeholder = hint;
    input.dataset.param = String(i);
    // Editing shows the stored value; creating shows the kind's sensible default so
    // the form can be saved without filling anything in.
    input.value = prefill.length ? (prefill[i] ?? '') : preset;

    wrap.append(lab, input);
    el.params.append(wrap);
  });
}

/// Turn a stored schedule back into the strings its kind's fields expect.
///
/// Returns `null` when the stored schedule is a different kind than the form is now
/// showing — which happens as soon as the user changes 提醒方式 while editing. The
/// caller must then fall back to defaults: filling an interval's "60" into a daily
/// time field produces "60", which parses as neither a time nor anything the user
/// meant.
function prefillFor(schedule) {
  const hm = (t) => `${String(t.hour).padStart(2, '0')}:${String(t.minute).padStart(2, '0')}`;
  const s = schedule ?? {};
  switch (s.scheduleType) {
    case 'interval': return [String(s.intervalMinutes ?? '')];
    case 'daily': return [hm(s.dailyTime)];
    case 'once': {
      if (!s.onceAt) return [''];
      const d = s.onceAt.date, t = s.onceAt.time;
      const p = (n) => String(n).padStart(2, '0');
      return [`${d.year}-${p(d.month)}-${p(d.day)} ${p(t.hour)}:${p(t.minute)}`];
    }
    case 'weekly': {
      const mask = s.weekdaysMask ?? 0;
      const days = [];
      for (let i = 0; i < 7; i++) if (mask & (1 << i)) days.push(i + 1);
      return [days.join(','), hm(s.dailyTime)];
    }
    case 'workdays': return [hm(s.dailyTime)];
    case 'monthly': {
      const mode = s.monthlyMode;
      const day = mode === 'lastDay' ? 'last' : mode === 'lastWorkday' ? 'lastwork' : String(s.monthlyDay ?? '');
      return [day, hm(s.dailyTime)];
    }
    case 'multiDaily': return [(s.dailyTimes ?? []).map(hm).join(',')];
    case 'timedInterval':
      return [hm(s.activeStart), hm(s.activeEnd), String(s.intervalMinutes ?? '')];
    case 'countdown': return [String(s.countdownMinutes ?? '')];
    case 'pomodoro':
      return [String(s.workMinutes ?? ''), String(s.breakMinutes ?? ''), String(s.cycles ?? '')];
    case 'afterStartup': return [String(s.startupDelayMinutes ?? '')];
    case 'usageBreak': return [String(s.usageMinutes ?? '')];
    default: return [];
  }
}

async function submit() {
  const params = [...el.params.querySelectorAll('input[data-param]')]
    .sort((a, b) => Number(a.dataset.param) - Number(b.dataset.param))
    .map((i) => i.value);

  const payload = {
    task: {
      id: editingId,
      title: el.title.value,
      message: el.message.value,
      kind: el.kind.value,
      params,
      enabled: el.enabled.checked,
    },
  };

  try {
    state = await invoke('save_task', payload);
    el.sheet.close();
    render();
  } catch (err) {
    // The backend returns the parse error verbatim, which is more useful than a
    // generic "invalid input".
    el.formError.textContent = String(err);
    el.formError.hidden = false;
  }
}

// -------------------------------------------------------------------- toasts

function showToast(fired) {
  const card = document.createElement('div');
  card.className = 'toast';

  let kicker = '到时间啦';
  let body = fired.message || '';
  if (fired.preAlert) {
    kicker = '马上开始';
  } else if (fired.usageSeconds > 0) {
    const mins = Math.round(fired.usageSeconds / 60);
    kicker = `已连续使用 ${mins} 分钟`;
    body = body || '起来走两步，让眼睛歇一会儿。';
  }

  const k = document.createElement('div');
  k.className = 'toast__kicker';
  k.textContent = kicker;

  const t = document.createElement('p');
  t.className = 'toast__title';
  t.textContent = fired.title;

  card.append(k, t);

  if (body) {
    const b = document.createElement('p');
    b.className = 'toast__body';
    b.textContent = body;
    card.append(b);
  }

  const foot = document.createElement('div');
  foot.className = 'toast__foot';
  const hint = document.createElement('span');
  hint.className = 'toast__dismiss';
  hint.textContent = fired.preAlert ? '稍后正式提醒' : '点击任意位置关闭';
  foot.append(hint);
  card.append(foot);

  // Clicking anywhere dismisses, and the card also fades on its own: a notice that
  // waits for a button press is worse than the event it reports.
  const dismiss = () => {
    card.classList.add('is-leaving');
    setTimeout(() => card.remove(), 200);
  };
  card.addEventListener('click', dismiss);
  const timer = setTimeout(dismiss, fired.preAlert ? 4000 : 8000);
  card.addEventListener('click', () => clearTimeout(timer), { once: true });

  el.stack.append(card);
  // Cap the stack: resuming after a long idle can fire many tasks at once, and an
  // unbounded column runs off the screen with the oldest cards unreachable. The
  // newest is the most useful, so the oldest goes first.
  while (el.stack.children.length > MAX_TOASTS) {
    el.stack.firstElementChild?.remove();
  }
}

// ---------------------------------------------------------------------- boot

async function boot() {
  const [kinds, autostart] = await Promise.all([
    invoke('describe_kinds'),
    invoke('get_autostart'),
  ]);

  // Fill the kind list, then fetch the parameter spec for the first one.
  el.kind.replaceChildren(
    ...kinds.map(([token, label]) => {
      const o = document.createElement('option');
      o.value = token;
      o.textContent = label;
      return o;
    }),
  );

  await Promise.all(
    kinds.map(async ([token]) => {
      paramSpecs.set(token, await invoke('param_labels', { kind: token }));
    }),
  );

  el.autostart.checked = autostart;
  state = await invoke('get_snapshot');
  render();

  if (state.loadError) {
    showToast({
      id: '',
      title: '配置读取失败',
      message: `${state.loadError}\n已以空列表启动，原文件未被覆盖。`,
      usageSeconds: 0,
      preAlert: false,
    });
  }
}

// -------------------------------------------------------------------- events

el.new.addEventListener('click', () => openSheet(null));
el.edit.addEventListener('click', () => {
  const task = state.tasks.find((t) => t.id === selected);
  if (task) openSheet(task);
});

el.toggle.addEventListener('click', async () => {
  const task = state.tasks.find((t) => t.id === selected);
  if (!task) return;
  await mutate('set_enabled', { id: task.id, enabled: !task.enabled }, '切换启用状态失败');
});

el.delete.addEventListener('click', async () => {
  if (!selected) return;
  const task = state.tasks.find((t) => t.id === selected);
  const ok = window.confirm(`删除「${task ? task.title : '这条提醒'}」？此操作不可撤销。`);
  if (!ok) return;
  if (await mutate('delete_task', { id: selected }, '删除失败')) {
    selected = null;
    render();
  }
});

el.close.addEventListener('click', () => {
  // Hiding is handled by the backend's CloseRequested hook; asking the window to
  // close runs that path, so the tray keeps running.
  window.__TAURI__.window.getCurrentWindow().close();
});

el.about.addEventListener('click', () => {
  showToast({
    id: '',
    title: '工作提醒助手 2.0.0',
    message: 'MIT 许可 · © 2026 Ma Xin\n12 种提醒方式 · 数据保存在本地，零网络请求。',
    usageSeconds: 0,
    preAlert: false,
  });
});

el.kind.addEventListener('change', () => rebuildParams(null));

el.autostart.addEventListener('change', async () => {
  try {
    const actual = await invoke('set_autostart', { enabled: el.autostart.checked });
    el.autostart.checked = actual;
  } catch (err) {
    el.autostart.checked = !el.autostart.checked;
    showToast({
      id: '',
      title: '开机自启设置失败',
      message: String(err),
      usageSeconds: 0,
      preAlert: false,
    });
  }
});

el.form.addEventListener('submit', (e) => {
  e.preventDefault();
  submit();
});

el.cancel.addEventListener('click', () => el.sheet.close());

// Backend pushes fired reminders from its own timer, so a notice still appears when
// the window is hidden in the tray.
listen('reminders-fired', (event) => {
  for (const fired of event.payload) showToast(fired);
});

// The list shows relative times, so refresh the snapshot when the window regains
// focus — otherwise a window left open overnight would show yesterday's times.
window.addEventListener('focus', async () => {
  try {
    state = await invoke('get_snapshot');
    render();
  } catch (err) {
    // A window activation must never throw; the list simply stays as it was.
    reportError('focus-refresh', err);
  }
});

boot();

// ----------------------------------------------------------------------- self-test
// `?selftest=<name>` runs a scripted interaction before the user sees the UI. It calls
// exactly the functions the buttons call, so it exercises the real IPC path rather
// than a mock. Used by tools/e2e_selftest.py; harmless in normal use because the
// parameter never appears in a shipped shortcut.
async function runSelfTest(name) {
  const log = (m) => {
    try { invoke('ui_log', { message: `selftest: ${m}` }); } catch (_) {}
  };
  log(`begin ${name}`);
  try {
    if (name === 'create') {
      openSheet(null);
      el.title.value = '测试任务A';
      el.message.value = '自检写入';
      await submit();
      log(`after submit, error visible=${!el.formError.hidden}`);
    } else if (name === 'switchKind') {
      // Create an interval task, then reopen it as a daily one: the old interval value
      // must not leak into the time field.
      openSheet(null);
      el.title.value = '切换类型';
      el.kind.value = 'interval';
      rebuildParams(null);
      await submit();
      const created = state.tasks.find((t) => t.title === '切换类型');
      if (!created) throw new Error('task not created');
      openSheet(created);
      el.kind.value = 'daily';
      rebuildParams(created.schedule);
      const field = el.params.querySelector('input[data-param]');
      log(`kind switch field value=${JSON.stringify(field && field.value)}`);
      await submit();
    }
    log('done');
  } catch (e) {
    log(`FAILED ${e && e.message ? e.message : e}`);
    reportError('selftest', e);
  }
}

// Exposed for the backend: it calls this only when started with --selftest=<name>.
window.__runSelfTest = runSelfTest;
