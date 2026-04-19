// Bottom Navigation Component

const NAV_ITEMS = [
  { id: 'contacts', icon: 'users', label: 'Contacts' },
  { id: 'chat', icon: 'message-circle', label: 'Chats' },
  { id: 'calendar', icon: 'calendar', label: 'Calendar' },
  { id: 'profile', icon: 'user', label: 'Profile' },
];

function renderNav(activeView) {
  const nav = document.getElementById('bottom-nav');

  const buttons = NAV_ITEMS.map(item => {
    const active = activeView === item.id;
    return `
      <button
        class="nav-item flex flex-col items-center gap-0.5 px-3 py-1 rounded-xl ${active ? 'text-indigo-600' : 'text-gray-400'}"
        data-nav="${item.id}"
      >
        <i data-lucide="${item.icon}" class="w-5 h-5"></i>
        <span class="text-[10px] font-medium">${escapeHtml(item.label)}</span>
      </button>
    `;
  }).join('');

  nav.innerHTML = '<div class="flex items-center justify-around py-2 px-2">' + buttons + '</div>';

  // Bind click events
  nav.querySelectorAll('[data-nav]').forEach(btn => {
    btn.addEventListener('click', () => {
      const view = btn.dataset.nav;
      if (view === 'chat') {
        navigate('chat', { list: true });
      } else {
        navigate(view);
      }
    });
  });

  lucide.createIcons();
}
