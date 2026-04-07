// Profile View

const MOCK_PROFILE = {
  name: 'Alexandru P.',
  phone: '+40 740 555 123',
  email: 'alex.p@email.com',
  role: 'both', // client, provider, both
  rating: 4.7,
  reviews: 34,
  completedServices: 28,
  joinedDate: 'March 2025',
  avatar: null, // No image, use initials
  services: ['Home Repairs', 'Furniture Assembly']
};

function renderProfile(params = {}) {
  const main = document.getElementById('main');

  const container = document.createElement('div');
  container.className = 'view-enter px-4 py-6';

  // Profile card
  const profileCard = document.createElement('div');
  profileCard.className = 'bg-white rounded-2xl shadow-sm p-6 text-center';

  // Avatar
  const initials = MOCK_PROFILE.name.split(' ').map(p => p[0]).join('').slice(0, 2).toUpperCase();
  const avatarDiv = document.createElement('div');
  avatarDiv.className = 'w-20 h-20 bg-indigo-500 rounded-full flex items-center justify-center text-white text-2xl font-bold mx-auto';
  avatarDiv.textContent = initials;
  profileCard.appendChild(avatarDiv);

  // Name
  const nameH = document.createElement('h2');
  nameH.className = 'text-lg font-semibold text-gray-900 mt-3';
  nameH.textContent = MOCK_PROFILE.name;
  profileCard.appendChild(nameH);

  // Phone
  const phoneP = document.createElement('p');
  phoneP.className = 'text-sm text-gray-500 mt-0.5';
  phoneP.textContent = MOCK_PROFILE.phone;
  profileCard.appendChild(phoneP);

  // Role badge
  const roleLabels = { client: 'Client', provider: 'Provider', both: 'Client & Provider' };
  const roleBadge = document.createElement('span');
  roleBadge.className = 'inline-block mt-2 text-xs px-3 py-1 rounded-full font-medium bg-indigo-100 text-indigo-700';
  roleBadge.textContent = roleLabels[MOCK_PROFILE.role] || 'Client';
  profileCard.appendChild(roleBadge);

  // Stats
  const statsDiv = document.createElement('div');
  statsDiv.className = 'flex items-center justify-center gap-6 mt-5 pt-5 border-t border-gray-100';

  const stats = [
    { value: MOCK_PROFILE.rating.toFixed(1), label: 'Rating', icon: 'star' },
    { value: MOCK_PROFILE.reviews, label: 'Reviews', icon: 'message-square' },
    { value: MOCK_PROFILE.completedServices, label: 'Completed', icon: 'check-circle' }
  ];

  stats.forEach(stat => {
    const statEl = document.createElement('div');
    statEl.className = 'text-center';
    const valP = document.createElement('p');
    valP.className = 'text-lg font-bold text-gray-900';
    valP.textContent = stat.value;
    statEl.appendChild(valP);
    const labelP = document.createElement('p');
    labelP.className = 'text-xs text-gray-400 mt-0.5';
    labelP.textContent = stat.label;
    statEl.appendChild(labelP);
    statsDiv.appendChild(statEl);
  });

  profileCard.appendChild(statsDiv);
  container.appendChild(profileCard);

  // Settings list
  const settingsCard = document.createElement('div');
  settingsCard.className = 'bg-white rounded-2xl shadow-sm mt-4 overflow-hidden';

  const settingsItems = [
    { icon: 'bell', label: 'Notifications', detail: 'On' },
    { icon: 'globe', label: 'Language', detail: 'English' },
    { icon: 'shield', label: 'Privacy', detail: '' },
    { icon: 'credit-card', label: 'Payment Methods', detail: '' },
    { icon: 'help-circle', label: 'Help & Support', detail: '' },
    { icon: 'info', label: 'About HelperBook', detail: 'v0.1.0' },
  ];

  settingsItems.forEach((item, idx) => {
    const row = document.createElement('div');
    row.className = 'flex items-center gap-3 px-4 py-3.5 cursor-pointer hover:bg-gray-50' +
      (idx < settingsItems.length - 1 ? ' border-b border-gray-50' : '');

    const iconWrap = document.createElement('div');
    iconWrap.className = 'w-8 h-8 bg-gray-50 rounded-lg flex items-center justify-center';
    const icon = document.createElement('i');
    icon.setAttribute('data-lucide', item.icon);
    icon.className = 'w-4 h-4 text-gray-500';
    iconWrap.appendChild(icon);
    row.appendChild(iconWrap);

    const labelSpan = document.createElement('span');
    labelSpan.className = 'flex-1 text-sm text-gray-900';
    labelSpan.textContent = item.label;
    row.appendChild(labelSpan);

    if (item.detail) {
      const detailSpan = document.createElement('span');
      detailSpan.className = 'text-xs text-gray-400';
      detailSpan.textContent = item.detail;
      row.appendChild(detailSpan);
    }

    const chevron = document.createElement('i');
    chevron.setAttribute('data-lucide', 'chevron-right');
    chevron.className = 'w-4 h-4 text-gray-300';
    row.appendChild(chevron);

    settingsCard.appendChild(row);
  });

  container.appendChild(settingsCard);

  // Joined date
  const joinedP = document.createElement('p');
  joinedP.className = 'text-xs text-gray-400 text-center mt-4';
  joinedP.textContent = 'Member since ' + MOCK_PROFILE.joinedDate;
  container.appendChild(joinedP);

  // Logout button
  const logoutBtn = document.createElement('button');
  logoutBtn.className = 'w-full mt-4 py-3 bg-white rounded-2xl shadow-sm text-sm font-medium text-red-500 hover:bg-red-50 transition-colors';
  logoutBtn.textContent = 'Log Out';
  logoutBtn.addEventListener('click', () => {
    // Placeholder
    alert('Logout functionality not yet implemented');
  });
  container.appendChild(logoutBtn);

  main.innerHTML = '';
  main.appendChild(container);
  lucide.createIcons();
}
