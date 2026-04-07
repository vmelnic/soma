// Main App Logic — Routing and State

const routes = {
  contacts: renderContacts,
  chat: renderChat,
  calendar: renderCalendar,
  profile: renderProfile
};

let currentView = 'contacts';

function navigate(view, params = {}) {
  currentView = view;

  // Restore header and nav visibility (chat view hides them)
  const header = document.getElementById('header');
  const bottomNav = document.getElementById('bottom-nav');
  if (view !== 'chat' || params.list) {
    header.style.display = '';
    bottomNav.style.display = '';
    document.getElementById('main').style.paddingBottom = '';
  }

  // Clear main content
  const main = document.getElementById('main');
  main.innerHTML = '';

  // Render view
  if (routes[view]) {
    routes[view](params);
  }

  // Update nav (use base view for nav highlighting)
  const navView = view;
  renderNav(navView);
}

// SOMA status polling
async function checkSomaStatus() {
  try {
    const status = await api.checkStatus();
    const dot = document.getElementById('soma-dot');
    const label = document.getElementById('soma-label');
    if (status.soma) {
      dot.className = 'w-2 h-2 rounded-full bg-green-400';
      label.className = 'text-xs text-green-600 font-medium';
      label.textContent = 'SOMA';
    } else {
      dot.className = 'w-2 h-2 rounded-full bg-gray-300';
      label.className = 'text-xs text-gray-400';
      label.textContent = 'SOMA';
    }
  } catch (e) {
    const dot = document.getElementById('soma-dot');
    const label = document.getElementById('soma-label');
    dot.className = 'w-2 h-2 rounded-full bg-gray-300';
    label.className = 'text-xs text-gray-400';
    label.textContent = 'SOMA';
  }
}

// Initialize app
document.addEventListener('DOMContentLoaded', () => {
  navigate('contacts');
  checkSomaStatus();
  // Poll status every 10 seconds
  setInterval(checkSomaStatus, 10000);
});
