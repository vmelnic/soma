// Contacts View

const MOCK_CONTACTS = [
  {
    id: '1', name: 'Ana M.', services: ['Hair Stylist', 'Makeup'],
    rating: 4.8, reviews: 124, distance: '0.8 km', online: true, verified: true,
    phone: '+40 721 123 456', role: 'provider',
    lastMessage: 'See you tomorrow at 3!', lastTime: '2m ago'
  },
  {
    id: '2', name: 'Ion P.', services: ['Plumber'],
    rating: 4.6, reviews: 89, distance: '1.2 km', online: false, verified: true,
    phone: '+40 722 234 567', role: 'provider',
    lastMessage: 'The pipe is fixed now.', lastTime: '1h ago'
  },
  {
    id: '3', name: 'Elena D.', services: ['House Cleaning', 'Laundry'],
    rating: 4.9, reviews: 203, distance: '2.1 km', online: true, verified: true,
    phone: '+40 723 345 678', role: 'provider',
    lastMessage: 'I can come on Saturday morning.', lastTime: '3h ago'
  },
  {
    id: '4', name: 'Mihai R.', services: ['Electrician'],
    rating: 4.5, reviews: 67, distance: '3.4 km', online: false, verified: false,
    phone: '+40 724 456 789', role: 'provider',
    lastMessage: 'What kind of installation?', lastTime: '1d ago'
  },
  {
    id: '5', name: 'Sofia L.', services: ['Massage', 'Physiotherapy'],
    rating: 4.7, reviews: 156, distance: '1.5 km', online: true, verified: true,
    phone: '+40 725 567 890', role: 'provider',
    lastMessage: 'Your next session is confirmed.', lastTime: '2d ago'
  },
  {
    id: '6', name: 'Andrei T.', services: ['Personal Trainer', 'Nutrition'],
    rating: 4.4, reviews: 45, distance: '4.0 km', online: false, verified: true,
    phone: '+40 726 678 901', role: 'provider',
    lastMessage: 'Great progress this week!', lastTime: '3d ago'
  },
  {
    id: '7', name: 'Maria V.', services: ['Tutoring', 'Translation'],
    rating: 5.0, reviews: 312, distance: '0.5 km', online: true, verified: true,
    phone: '+40 727 789 012', role: 'provider',
    lastMessage: 'The document is ready.', lastTime: '5d ago'
  },
  {
    id: '8', name: 'Cristian B.', services: ['Photography'],
    rating: 4.3, reviews: 38, distance: '5.2 km', online: false, verified: false,
    phone: '+40 728 890 123', role: 'provider',
    lastMessage: 'I sent you the photos.', lastTime: '1w ago'
  }
];

const DISCOVER_CONTACTS = [
  {
    id: '9', name: 'Raluca S.', services: ['Interior Design'],
    rating: 4.9, reviews: 78, distance: '2.8 km', online: true, verified: true,
    phone: '+40 729 901 234', role: 'provider'
  },
  {
    id: '10', name: 'Dan C.', services: ['Carpentry', 'Furniture'],
    rating: 4.7, reviews: 92, distance: '6.1 km', online: false, verified: true,
    phone: '+40 730 012 345', role: 'provider'
  },
  {
    id: '11', name: 'Ioana F.', services: ['Pet Sitting', 'Dog Walking'],
    rating: 4.8, reviews: 145, distance: '1.0 km', online: true, verified: true,
    phone: '+40 731 123 456', role: 'provider'
  },
  {
    id: '12', name: 'Victor N.', services: ['Auto Mechanic'],
    rating: 4.6, reviews: 210, distance: '3.7 km', online: false, verified: true,
    phone: '+40 732 234 567', role: 'provider'
  }
];

async function loadContacts() {
  try {
    const result = await api.find({ table: 'contacts', filter: {} });
    if (result.result && result.result.content) {
      return JSON.parse(result.result.content[0].text);
    }
  } catch (e) {
    // Fall back to mock data
  }
  return MOCK_CONTACTS;
}

function renderContactsList(contacts, listEl) {
  listEl.innerHTML = '';
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

  const contacts = activeTab === 'my' ? MOCK_CONTACTS : DISCOVER_CONTACTS;
  renderContactsList(contacts, listEl);

  // Search filter
  const searchInput = document.getElementById('contact-search');
  searchInput.addEventListener('input', (e) => {
    const query = e.target.value.toLowerCase();
    const filtered = contacts.filter(c =>
      c.name.toLowerCase().includes(query) ||
      c.services.some(s => s.toLowerCase().includes(query))
    );
    renderContactsList(filtered, listEl);
  });

  lucide.createIcons();
}
