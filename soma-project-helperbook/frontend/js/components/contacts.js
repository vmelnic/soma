// Contacts View

async function loadContacts() {
  try {
    const result = await api.query(
      "SELECT u.id, u.name, u.phone, u.role, u.bio, u.is_verified, " +
      "u.location_lat, u.location_lon, " +
      "COALESCE(pp.service_area_radius, 25) as radius " +
      "FROM users u LEFT JOIN provider_profiles pp ON pp.user_id = u.id " +
      "WHERE u.role IN ('provider', 'both') " +
      "ORDER BY u.name LIMIT 50"
    );
    const rows = SomaAPI.extractRows(result);
    if (rows && Array.isArray(rows) && rows.length > 0) {
      return rows.map(mapDbUserToContact);
    }
  } catch (e) {
    console.warn('[contacts] API load failed:', e.message);
  }
  return [];
}

async function loadDiscoverContacts() {
  try {
    const result = await api.query(
      "SELECT u.id, u.name, u.phone, u.role, u.bio, u.is_verified, " +
      "u.location_lat, u.location_lon " +
      "FROM users u " +
      "WHERE u.role IN ('provider', 'both') " +
      "AND u.id NOT IN (SELECT recipient_id FROM connections WHERE requester_id = 1 AND status = 'accepted') " +
      "ORDER BY u.created_at DESC LIMIT 20"
    );
    const rows = SomaAPI.extractRows(result);
    if (rows && Array.isArray(rows) && rows.length > 0) {
      return rows.map(mapDbUserToContact);
    }
  } catch (e) {
    console.warn('[contacts] API discover load failed:', e.message);
  }
  return [];
}

function mapDbUserToContact(row) {
  // Row comes from the database — field names match column names
  const name = row.name || 'Unknown';
  const bio = row.bio || '';
  // Derive services from bio (split on commas or use as single service)
  const services = bio ? bio.split(',').map(s => s.trim()).filter(Boolean) : [];
  return {
    id: String(row.id),
    name: name,
    phone: row.phone || '',
    role: row.role || 'provider',
    bio: bio,
    services: services.length ? services : ['General'],
    rating: 0,
    reviews: 0,
    distance: '',
    online: false,
    verified: !!row.is_verified,
    lastMessage: '',
    lastTime: ''
  };
}

function renderContactsList(contacts, listEl) {
  listEl.innerHTML = '';
  if (!contacts || contacts.length === 0) {
    listEl.innerHTML = '<div class="text-center py-8 text-gray-400 text-sm">No contacts yet</div>';
    return;
  }
  contacts.forEach(contact => {
    const card = renderProviderCard(contact, (c) => {
      navigate('chat', { contact: c });
    });
    listEl.appendChild(card);
  });
  lucide.createIcons();
}

function renderContacts(params = {}) {
  const main = document.getElementById('main');
  const activeTab = params.tab || 'my';

  const container = document.createElement('div');
  container.className = 'view-enter';

  // Search bar
  const searchWrap = document.createElement('div');
  searchWrap.className = 'px-4 pt-4 pb-2';
  searchWrap.innerHTML = `
    <div class="relative">
      <i data-lucide="search" class="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-gray-400"></i>
      <input
        type="text"
        id="contact-search"
        placeholder="Search contacts or services..."
        class="w-full pl-10 pr-4 py-2.5 bg-white rounded-xl border border-gray-200 text-sm placeholder:text-gray-400 focus:border-indigo-300"
      >
    </div>
  `;
  container.appendChild(searchWrap);

  // Tabs
  const tabsWrap = document.createElement('div');
  tabsWrap.className = 'flex border-b border-gray-100 px-4';
  const tabMy = document.createElement('button');
  tabMy.className = 'flex-1 py-3 text-sm font-medium text-center ' + (activeTab === 'my' ? 'text-indigo-600 tab-active' : 'text-gray-400');
  tabMy.textContent = 'My Contacts';
  tabMy.addEventListener('click', () => renderContacts({ tab: 'my' }));
  const tabDiscover = document.createElement('button');
  tabDiscover.className = 'flex-1 py-3 text-sm font-medium text-center ' + (activeTab === 'discover' ? 'text-indigo-600 tab-active' : 'text-gray-400');
  tabDiscover.textContent = 'Discover';
  tabDiscover.addEventListener('click', () => renderContacts({ tab: 'discover' }));
  tabsWrap.appendChild(tabMy);
  tabsWrap.appendChild(tabDiscover);
  container.appendChild(tabsWrap);

  // Contact list
  const listEl = document.createElement('div');
  listEl.id = 'contacts-list';
  listEl.className = 'px-4 py-3 flex flex-col gap-2';
  container.appendChild(listEl);

  main.innerHTML = '';
  main.appendChild(container);

  // Show loading state
  listEl.innerHTML = '<div class="text-center py-8 text-gray-400 text-sm">Loading...</div>';
  lucide.createIcons();

  // Load contacts asynchronously
  const contactsPromise = activeTab === 'my' ? loadContacts() : loadDiscoverContacts();
  contactsPromise.then(contacts => {
    renderContactsList(contacts, listEl);

    // Search filter
    const searchInput = document.getElementById('contact-search');
    if (searchInput) {
      searchInput.addEventListener('input', (e) => {
        const query = e.target.value.toLowerCase();
        const filtered = contacts.filter(c =>
          c.name.toLowerCase().includes(query) ||
          c.services.some(s => s.toLowerCase().includes(query))
        );
        renderContactsList(filtered, listEl);
      });
    }
  });
}
