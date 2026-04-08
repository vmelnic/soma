// Chat View

const MOCK_MESSAGES = {
  '1': [
    { id: 'm1', from: 'them', text: 'Hi! I saw your profile. Do you have availability this week?', time: '10:30 AM' },
    { id: 'm2', from: 'me', text: 'Yes! I have slots on Wednesday and Friday afternoon.', time: '10:32 AM' },
    { id: 'm3', from: 'them', text: 'Wednesday at 3 PM works for me.', time: '10:33 AM' },
    { id: 'm4', from: 'me', text: 'Perfect. I will book you in.', time: '10:35 AM' },
    {
      id: 'm5', type: 'appointment',
      service: 'Hair Styling',
      date: 'Wed, Apr 9',
      time: '3:00 PM',
      status: 'confirmed',
      provider: 'Ana M.'
    },
    { id: 'm6', from: 'them', text: 'See you tomorrow at 3!', time: '2:15 PM' },
  ],
  '2': [
    { id: 'm1', from: 'me', text: 'Hi Ion, I have a leaky faucet in the kitchen. Can you help?', time: '9:00 AM' },
    { id: 'm2', from: 'them', text: 'Sure, I can come today between 2-4 PM. Does that work?', time: '9:15 AM' },
    { id: 'm3', from: 'me', text: 'That works. Address is Str. Victoriei 45.', time: '9:16 AM' },
    { id: 'm4', from: 'them', text: 'On my way!', time: '1:55 PM' },
    { id: 'm5', from: 'them', text: 'The pipe is fixed now.', time: '3:20 PM' },
  ],
  '3': [
    { id: 'm1', from: 'me', text: 'Hello Elena, do you offer deep cleaning services?', time: 'Mon 8:00 AM' },
    { id: 'm2', from: 'them', text: 'Yes! I do regular and deep cleaning. For deep cleaning the rate is 200 RON for a 2-bedroom apartment.', time: 'Mon 8:30 AM' },
    { id: 'm3', from: 'me', text: 'That sounds good. When is the earliest you can come?', time: 'Mon 8:32 AM' },
    { id: 'm4', from: 'them', text: 'I can come on Saturday morning.', time: 'Mon 9:00 AM' },
  ],
  '5': [
    { id: 'm1', from: 'them', text: 'Hello! Ready for your massage session?', time: 'Tue 4:00 PM' },
    { id: 'm2', from: 'me', text: 'Yes, same time as usual please.', time: 'Tue 4:05 PM' },
    {
      id: 'm3', type: 'appointment',
      service: 'Deep Tissue Massage',
      date: 'Thu, Apr 10',
      time: '5:00 PM',
      status: 'confirmed',
      provider: 'Sofia L.'
    },
    { id: 'm4', from: 'them', text: 'Your next session is confirmed.', time: 'Tue 4:10 PM' },
  ]
};

// Build a default empty conversation for contacts without specific messages
function getMessages(contactId) {
  return MOCK_MESSAGES[contactId] || [
    { id: 'm0', from: 'them', text: 'Hi there! How can I help you?', time: 'Recently' }
  ];
}

// Current user ID (fixed for now — would come from auth in production)
function getCurrentUserId() {
  return window.SOMA_USER_ID || 'unknown';
}

async function loadChatContacts() {
  try {
    // Load contacts that have chats with the current user
    const myId = getCurrentUserId();
    const result = await api.query(
      `SELECT DISTINCT u.id, u.name, u.phone, u.role, u.bio, u.is_verified
       FROM users u
       JOIN chat_members cm1 ON cm1.user_id = u.id
       JOIN chat_members cm2 ON cm2.chat_id = cm1.chat_id
       WHERE cm2.user_id = '${myId}' AND u.id != '${myId}'
       ORDER BY u.name`
    );
    const rows = SomaAPI.extractRows(result);
    if (rows && Array.isArray(rows) && rows.length > 0) {
      return rows.map(row => ({
        id: String(row.id),
        name: row.name || 'Unknown',
        phone: row.phone || '',
        role: row.role || 'provider',
        services: (row.bio || '').split(',').map(s => s.trim()).filter(Boolean),
        online: false,
        verified: !!row.is_verified,
        lastMessage: row.last_message || '',
        lastTime: formatTimeAgo(row.last_time)
      }));
    }
  } catch (e) {
    console.warn('[chat] API load failed, using mock data:', e.message);
  }
  return null; // null means use mock data
}

async function loadChatMessages(contactId) {
  try {
    const myId = getCurrentUserId();
    const result = await api.query(
      `SELECT m.id, m.sender_id, m.type, m.content, m.status, m.created_at
       FROM messages m
       WHERE m.chat_id IN (
         SELECT cm1.chat_id FROM chat_members cm1
         JOIN chat_members cm2 ON cm2.chat_id = cm1.chat_id
         WHERE cm1.user_id = '${myId}' AND cm2.user_id = '${contactId}'
       )
       ORDER BY m.created_at ASC LIMIT 100`
    );
    const rows = SomaAPI.extractRows(result);
    if (rows && Array.isArray(rows) && rows.length > 0) {
      return rows.map(row => {
        const isMe = String(row.sender_id) === String(getCurrentUserId());
        if (row.type === 'appointment') {
          return {
            id: String(row.id),
            type: 'appointment',
            service: row.content || 'Appointment',
            date: '',
            time: '',
            status: row.status || 'confirmed',
            provider: ''
          };
        }
        return {
          id: String(row.id),
          from: isMe ? 'me' : 'them',
          text: row.content || '',
          time: formatMessageTime(row.created_at)
        };
      });
    }
  } catch (e) {
    console.warn('[chat] Failed to load messages for contact ' + contactId + ':', e.message);
  }
  return null; // null means use mock data
}

function formatTimeAgo(timestamp) {
  if (!timestamp) return '';
  try {
    const date = new Date(timestamp);
    const now = new Date();
    const diffMs = now - date;
    const diffMins = Math.floor(diffMs / 60000);
    if (diffMins < 1) return 'now';
    if (diffMins < 60) return diffMins + 'm ago';
    const diffHours = Math.floor(diffMins / 60);
    if (diffHours < 24) return diffHours + 'h ago';
    const diffDays = Math.floor(diffHours / 24);
    if (diffDays < 7) return diffDays + 'd ago';
    return Math.floor(diffDays / 7) + 'w ago';
  } catch (e) {
    return '';
  }
}

function formatMessageTime(timestamp) {
  if (!timestamp) return '';
  try {
    const date = new Date(timestamp);
    return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  } catch (e) {
    return '';
  }
}

function renderChatList() {
  const main = document.getElementById('main');
  const container = document.createElement('div');
  container.className = 'view-enter';

  // Title
  const title = document.createElement('div');
  title.className = 'px-4 pt-4 pb-2';
  const h2 = document.createElement('h2');
  h2.className = 'text-lg font-semibold text-gray-900';
  h2.textContent = 'Messages';
  title.appendChild(h2);
  container.appendChild(title);

  const list = document.createElement('div');
  list.className = 'px-4 flex flex-col';
  list.innerHTML = '<div class="text-center py-8 text-gray-400 text-sm">Loading...</div>';
  container.appendChild(list);

  main.innerHTML = '';
  main.appendChild(container);

  // Try loading from API, fall back to mock
  loadChatContacts().then(apiContacts => {
    const chatContacts = apiContacts || MOCK_CONTACTS.filter(c => MOCK_MESSAGES[c.id]);
    list.innerHTML = '';

    chatContacts.forEach(contact => {
      const mockMessages = MOCK_MESSAGES[contact.id];
      const lastText = contact.lastMessage
        || (mockMessages ? (mockMessages[mockMessages.length - 1].type === 'appointment'
            ? 'Appointment: ' + mockMessages[mockMessages.length - 1].service
            : mockMessages[mockMessages.length - 1].text)
          : '');
      const lastTime = contact.lastTime || '';

      const row = document.createElement('div');
      row.className = 'flex items-center gap-3 py-3 border-b border-gray-50 cursor-pointer hover:bg-gray-50 rounded-lg px-2 -mx-2';
      row.addEventListener('click', () => navigate('chat', { contact }));

      // Avatar
      const colors = [
        'bg-indigo-500', 'bg-rose-500', 'bg-emerald-500', 'bg-amber-500',
        'bg-cyan-500', 'bg-purple-500', 'bg-pink-500', 'bg-teal-500'
      ];
      const hash = contact.name.split('').reduce((a, c) => a + c.charCodeAt(0), 0);
      const avatarColor = colors[hash % colors.length];
      const initials = contact.name.split(' ').map(p => p[0]).join('').slice(0, 2).toUpperCase();

      const avatar = document.createElement('div');
      avatar.className = 'relative flex-shrink-0';
      const circle = document.createElement('div');
      circle.className = 'w-12 h-12 ' + avatarColor + ' rounded-full flex items-center justify-center text-white font-semibold text-sm';
      circle.textContent = initials;
      avatar.appendChild(circle);
      if (contact.online) {
        const dot = document.createElement('span');
        dot.className = 'absolute bottom-0 right-0 w-3 h-3 bg-green-400 rounded-full border-2 border-white status-online';
        avatar.appendChild(dot);
      }
      row.appendChild(avatar);

      // Text content
      const textDiv = document.createElement('div');
      textDiv.className = 'flex-1 min-w-0';
      const nameRow = document.createElement('div');
      nameRow.className = 'flex items-center justify-between';
      const nameSpan = document.createElement('span');
      nameSpan.className = 'font-semibold text-sm text-gray-900';
      nameSpan.textContent = contact.name;
      const timeSpan = document.createElement('span');
      timeSpan.className = 'text-xs text-gray-400';
      timeSpan.textContent = lastTime;
      nameRow.appendChild(nameSpan);
      nameRow.appendChild(timeSpan);
      textDiv.appendChild(nameRow);

      const msgP = document.createElement('p');
      msgP.className = 'text-sm text-gray-500 truncate mt-0.5';
      msgP.textContent = lastText;
      textDiv.appendChild(msgP);

      row.appendChild(textDiv);
      list.appendChild(row);
    });
  });
}

function renderChat(params = {}) {
  if (params.list || !params.contact) {
    renderChatList();
    return;
  }

  const contact = params.contact;
  const main = document.getElementById('main');
  // Start with mock messages, then try to load from API
  let messages = getMessages(contact.id);

  // Hide default header, show chat header
  const header = document.getElementById('header');
  header.style.display = 'none';

  // Hide bottom nav
  const bottomNav = document.getElementById('bottom-nav');
  bottomNav.style.display = 'none';

  const container = document.createElement('div');
  container.className = 'view-enter flex flex-col h-screen';

  // Chat header
  const chatHeader = document.createElement('div');
  chatHeader.className = 'bg-white border-b border-gray-100 px-4 py-3 flex items-center gap-3 flex-shrink-0';

  const backBtn = document.createElement('button');
  backBtn.className = 'p-1 -ml-1 text-gray-500 hover:text-gray-700';
  backBtn.innerHTML = '<i data-lucide="arrow-left" class="w-5 h-5"></i>';
  backBtn.addEventListener('click', () => {
    header.style.display = '';
    bottomNav.style.display = '';
    navigate('chat', { list: true });
  });
  chatHeader.appendChild(backBtn);

  // Avatar
  const colors = [
    'bg-indigo-500', 'bg-rose-500', 'bg-emerald-500', 'bg-amber-500',
    'bg-cyan-500', 'bg-purple-500', 'bg-pink-500', 'bg-teal-500'
  ];
  const hash = contact.name.split('').reduce((a, c) => a + c.charCodeAt(0), 0);
  const avatarColor = colors[hash % colors.length];
  const initials = contact.name.split(' ').map(p => p[0]).join('').slice(0, 2).toUpperCase();

  const avatarDiv = document.createElement('div');
  avatarDiv.className = 'w-9 h-9 ' + avatarColor + ' rounded-full flex items-center justify-center text-white font-medium text-xs';
  avatarDiv.textContent = initials;
  chatHeader.appendChild(avatarDiv);

  const nameDiv = document.createElement('div');
  nameDiv.className = 'flex-1';
  const nameP = document.createElement('p');
  nameP.className = 'font-semibold text-sm text-gray-900';
  nameP.textContent = contact.name;
  nameDiv.appendChild(nameP);
  const statusP = document.createElement('p');
  statusP.className = 'text-xs ' + (contact.online ? 'text-green-500' : 'text-gray-400');
  statusP.textContent = contact.online ? 'Online' : 'Offline';
  nameDiv.appendChild(statusP);
  chatHeader.appendChild(nameDiv);

  const phoneBtn = document.createElement('button');
  phoneBtn.className = 'p-2 text-gray-400 hover:text-indigo-600';
  phoneBtn.innerHTML = '<i data-lucide="phone" class="w-5 h-5"></i>';
  chatHeader.appendChild(phoneBtn);

  container.appendChild(chatHeader);

  // Messages area
  const messagesArea = document.createElement('div');
  messagesArea.className = 'flex-1 overflow-y-auto px-4 py-3';
  messagesArea.id = 'chat-messages';

  const messagesInner = document.createElement('div');
  messagesInner.className = 'chat-messages flex flex-col gap-2';

  messages.forEach(msg => {
    if (msg.type === 'appointment') {
      // Appointment card
      const card = document.createElement('div');
      card.className = 'bg-indigo-50 rounded-2xl p-4 my-2 border border-indigo-100';
      
      const cardTitle = document.createElement('div');
      cardTitle.className = 'flex items-center gap-2 mb-2';
      const calIcon = document.createElement('i');
      calIcon.setAttribute('data-lucide', 'calendar-check');
      calIcon.className = 'w-4 h-4 text-indigo-600';
      cardTitle.appendChild(calIcon);
      const titleSpan = document.createElement('span');
      titleSpan.className = 'text-sm font-semibold text-indigo-700';
      titleSpan.textContent = 'Appointment Booked';
      cardTitle.appendChild(titleSpan);
      card.appendChild(cardTitle);

      const serviceP = document.createElement('p');
      serviceP.className = 'text-sm font-medium text-gray-900';
      serviceP.textContent = msg.service;
      card.appendChild(serviceP);

      const detailP = document.createElement('p');
      detailP.className = 'text-xs text-gray-500 mt-1';
      detailP.textContent = msg.date + ' at ' + msg.time;
      card.appendChild(detailP);

      const statusBadge = document.createElement('span');
      statusBadge.className = 'inline-block mt-2 text-xs px-2 py-0.5 rounded-full font-medium badge-' + msg.status;
      statusBadge.textContent = msg.status.charAt(0).toUpperCase() + msg.status.slice(1);
      card.appendChild(statusBadge);

      messagesInner.appendChild(card);
    } else {
      // Regular message bubble
      const isMe = msg.from === 'me';
      const bubble = document.createElement('div');
      bubble.className = 'flex ' + (isMe ? 'justify-end' : 'justify-start');

      const inner = document.createElement('div');
      inner.className = 'max-w-[75%] px-4 py-2.5 rounded-2xl text-sm ' +
        (isMe
          ? 'bg-indigo-600 text-white bubble-sent'
          : 'bg-white text-gray-900 shadow-sm bubble-received');

      const textP = document.createElement('p');
      textP.textContent = msg.text;
      inner.appendChild(textP);

      const timeSpan = document.createElement('p');
      timeSpan.className = 'text-[10px] mt-1 ' + (isMe ? 'text-indigo-200' : 'text-gray-400');
      timeSpan.textContent = msg.time;
      inner.appendChild(timeSpan);

      bubble.appendChild(inner);
      messagesInner.appendChild(bubble);
    }
  });

  messagesArea.appendChild(messagesInner);
  container.appendChild(messagesArea);

  // Input bar
  const inputBar = document.createElement('div');
  inputBar.className = 'bg-white border-t border-gray-100 px-4 py-3 flex items-center gap-2 flex-shrink-0';

  const textInput = document.createElement('input');
  textInput.type = 'text';
  textInput.placeholder = 'Type a message...';
  textInput.className = 'flex-1 px-4 py-2.5 bg-gray-50 rounded-xl text-sm placeholder:text-gray-400 border border-gray-200 focus:border-indigo-300';
  textInput.id = 'chat-input';
  inputBar.appendChild(textInput);

  const sendBtn = document.createElement('button');
  sendBtn.className = 'w-10 h-10 bg-indigo-600 rounded-xl flex items-center justify-center text-white hover:bg-indigo-700 transition-colors flex-shrink-0';
  const sendIcon = document.createElement('i');
  sendIcon.setAttribute('data-lucide', 'send');
  sendIcon.className = 'w-4 h-4';
  sendBtn.appendChild(sendIcon);
  sendBtn.addEventListener('click', () => sendMessage(contact));
  inputBar.appendChild(sendBtn);

  container.appendChild(inputBar);

  main.innerHTML = '';
  main.style.paddingBottom = '0';
  main.appendChild(container);

  lucide.createIcons();

  // Scroll to bottom
  messagesArea.scrollTop = messagesArea.scrollHeight;

  // Enter key to send
  textInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') sendMessage(contact);
  });
  textInput.focus();

  // Try loading messages from API in the background
  loadChatMessages(contact.id).then(apiMessages => {
    if (apiMessages && apiMessages.length > 0) {
      // Replace mock messages with real ones
      messagesInner.innerHTML = '';
      apiMessages.forEach(msg => {
        appendMessageBubble(messagesInner, msg);
      });
      lucide.createIcons();
      messagesArea.scrollTop = messagesArea.scrollHeight;
    }
  });
}

function appendMessageBubble(container, msg) {
  if (msg.type === 'appointment') {
    const card = document.createElement('div');
    card.className = 'bg-indigo-50 rounded-2xl p-4 my-2 border border-indigo-100';

    const cardTitle = document.createElement('div');
    cardTitle.className = 'flex items-center gap-2 mb-2';
    const calIcon = document.createElement('i');
    calIcon.setAttribute('data-lucide', 'calendar-check');
    calIcon.className = 'w-4 h-4 text-indigo-600';
    cardTitle.appendChild(calIcon);
    const titleSpan = document.createElement('span');
    titleSpan.className = 'text-sm font-semibold text-indigo-700';
    titleSpan.textContent = 'Appointment Booked';
    cardTitle.appendChild(titleSpan);
    card.appendChild(cardTitle);

    const serviceP = document.createElement('p');
    serviceP.className = 'text-sm font-medium text-gray-900';
    serviceP.textContent = msg.service;
    card.appendChild(serviceP);

    if (msg.date || msg.time) {
      const detailP = document.createElement('p');
      detailP.className = 'text-xs text-gray-500 mt-1';
      detailP.textContent = (msg.date || '') + (msg.date && msg.time ? ' at ' : '') + (msg.time || '');
      card.appendChild(detailP);
    }

    const statusBadge = document.createElement('span');
    statusBadge.className = 'inline-block mt-2 text-xs px-2 py-0.5 rounded-full font-medium badge-' + (msg.status || 'confirmed');
    statusBadge.textContent = (msg.status || 'confirmed').charAt(0).toUpperCase() + (msg.status || 'confirmed').slice(1);
    card.appendChild(statusBadge);

    container.appendChild(card);
  } else {
    const isMe = msg.from === 'me';
    const bubble = document.createElement('div');
    bubble.className = 'flex ' + (isMe ? 'justify-end' : 'justify-start');

    const inner = document.createElement('div');
    inner.className = 'max-w-[75%] px-4 py-2.5 rounded-2xl text-sm ' +
      (isMe
        ? 'bg-indigo-600 text-white bubble-sent'
        : 'bg-white text-gray-900 shadow-sm bubble-received');

    const textP = document.createElement('p');
    textP.textContent = msg.text;
    inner.appendChild(textP);

    const timeSpan = document.createElement('p');
    timeSpan.className = 'text-[10px] mt-1 ' + (isMe ? 'text-indigo-200' : 'text-gray-400');
    timeSpan.textContent = msg.time;
    inner.appendChild(timeSpan);

    bubble.appendChild(inner);
    container.appendChild(bubble);
  }
}

async function sendMessage(contact) {
  const input = document.getElementById('chat-input');
  const text = input.value.trim();
  if (!text) return;

  input.value = '';

  const messagesEl = document.querySelector('.chat-messages');
  const now = new Date();
  const timeStr = now.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });

  // Show bubble immediately (optimistic UI)
  appendMessageBubble(messagesEl, {
    from: 'me',
    text: text,
    time: timeStr,
    type: 'text'
  });

  const messagesArea = document.getElementById('chat-messages');
  messagesArea.scrollTop = messagesArea.scrollHeight;

  // Persist to database via SOMA
  try {
    const myId = getCurrentUserId();
    const contactId = contact.id;

    // Find or create a direct chat between these users
    let chatId = null;
    const chatResult = await api.query(
      `SELECT cm1.chat_id FROM chat_members cm1
       JOIN chat_members cm2 ON cm2.chat_id = cm1.chat_id
       JOIN chats c ON c.id = cm1.chat_id
       WHERE cm1.user_id = '${myId}' AND cm2.user_id = '${contactId}'
       AND c.type = 'direct' LIMIT 1`
    );
    const chatRows = SomaAPI.extractRows(chatResult);
    if (chatRows && chatRows.length > 0) {
      chatId = chatRows[0].chat_id;
    } else {
      // Create new chat
      await api.execute(
        `INSERT INTO chats (type, created_by) VALUES ('direct', '${myId}')`
      );
      const newChat = await api.query(
        `SELECT id FROM chats WHERE created_by = '${myId}' ORDER BY created_at DESC LIMIT 1`
      );
      const newRows = SomaAPI.extractRows(newChat);
      if (newRows && newRows.length > 0) {
        chatId = newRows[0].id;
        // Add both users as members
        await api.execute(
          `INSERT INTO chat_members (chat_id, user_id) VALUES ('${chatId}', '${myId}'), ('${chatId}', '${contactId}')`
        );
      }
    }

    if (chatId) {
      // Escape single quotes in message text
      const escaped = text.replace(/'/g, "''");
      await api.execute(
        `INSERT INTO messages (chat_id, sender_id, type, content, status)
         VALUES ('${chatId}', '${myId}', 'text', '${escaped}', 'sent')`
      );
      console.log('[chat] Message persisted to database');
    }
  } catch (e) {
    console.warn('[chat] Failed to persist message:', e.message);
    // Message is still shown in UI — will sync on next load
  }
}
