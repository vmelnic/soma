// Provider/Contact Card Component

function renderProviderCard(contact, onClick) {
  const card = document.createElement('div');
  card.className = 'bg-white rounded-2xl shadow-sm p-4 flex items-center gap-3 cursor-pointer hover:shadow-md transition-shadow';
  if (onClick) {
    card.addEventListener('click', () => onClick(contact));
  }

  // Avatar colors based on name hash
  const colors = [
    'bg-indigo-500', 'bg-rose-500', 'bg-emerald-500', 'bg-amber-500',
    'bg-cyan-500', 'bg-purple-500', 'bg-pink-500', 'bg-teal-500'
  ];
  const hash = contact.name.split('').reduce((a, c) => a + c.charCodeAt(0), 0);
  const avatarColor = colors[hash % colors.length];

  // Initials
  const parts = contact.name.split(' ');
  const initials = parts.map(p => p[0]).join('').slice(0, 2).toUpperCase();

  // Stars
  const rating = contact.rating || 0;
  let stars = '';
  for (let i = 1; i <= 5; i++) {
    if (i <= Math.floor(rating)) {
      stars += '<i data-lucide="star" class="w-3.5 h-3.5 text-yellow-400 fill-yellow-400"></i>';
    } else if (i - 0.5 <= rating) {
      stars += '<i data-lucide="star-half" class="w-3.5 h-3.5 text-yellow-400 fill-yellow-400"></i>';
    } else {
      stars += '<i data-lucide="star" class="w-3.5 h-3.5 text-gray-200"></i>';
    }
  }

  // Service tags
  const tags = (contact.services || []).map(s =>
    '<span class="text-xs bg-indigo-50 text-indigo-600 px-2 py-0.5 rounded-full font-medium">' + escapeHtml(s) + '</span>'
  ).join('');

  // Online status
  const statusDot = contact.online
    ? '<span class="absolute bottom-0 right-0 w-3 h-3 bg-green-400 rounded-full border-2 border-white status-online"></span>'
    : '';

  card.innerHTML = `
    <div class="relative flex-shrink-0">
      <div class="w-12 h-12 ${avatarColor} rounded-full flex items-center justify-center text-white font-semibold text-sm">
        ${escapeHtml(initials)}
      </div>
      ${statusDot}
    </div>
    <div class="flex-1 min-w-0">
      <div class="flex items-center gap-2">
        <span class="font-semibold text-sm text-gray-900 truncate">${escapeHtml(contact.name)}</span>
        ${contact.verified ? '<i data-lucide="badge-check" class="w-4 h-4 text-indigo-500 flex-shrink-0"></i>' : ''}
      </div>
      <div class="flex items-center gap-1 mt-0.5">${stars}
        <span class="text-xs text-gray-400 ml-1">${rating.toFixed(1)}</span>
      </div>
      <div class="flex flex-wrap gap-1 mt-1.5">${tags}</div>
    </div>
    <div class="flex flex-col items-end gap-1 flex-shrink-0">
      ${contact.distance ? '<span class="text-xs text-gray-400">' + escapeHtml(contact.distance) + '</span>' : ''}
      <i data-lucide="chevron-right" class="w-4 h-4 text-gray-300"></i>
    </div>
  `;

  return card;
}

// HTML escaping utility
function escapeHtml(str) {
  const div = document.createElement('div');
  div.textContent = str;
  return div.innerHTML;
}
