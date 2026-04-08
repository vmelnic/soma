// Main App Logic — Routing and State

const routes = {
  contacts: renderContacts,
  chat: renderChat,
  calendar: renderCalendar,
  profile: renderProfile
};

let currentView = 'contacts';

// Current user ID — matches the deterministic seed UUID in seed.sql
window.SOMA_USER_ID = '00000000-0000-0000-0000-000000000001';

function navigate(view, params) {
  params = params || {};
  currentView = view;

  // Restore header and nav visibility (chat view hides them)
  var header = document.getElementById('header');
  var bottomNav = document.getElementById('bottom-nav');
  if (view !== 'chat' || params.list) {
    header.style.display = '';
    bottomNav.style.display = '';
    document.getElementById('main').style.paddingBottom = '';
  }

  var main = document.getElementById('main');
  main.textContent = '';

  if (routes[view]) {
    routes[view](params);
  }

  renderNav(view);
}

// SOMA status polling
function checkSomaStatus() {
  api.checkStatus().then(function(status) {
    var dot = document.getElementById('soma-dot');
    var label = document.getElementById('soma-label');
    if (status.soma) {
      dot.className = 'w-2 h-2 rounded-full bg-green-400';
      label.className = 'text-xs text-green-600 font-medium';
      label.textContent = 'SOMA';
    } else {
      dot.className = 'w-2 h-2 rounded-full bg-gray-300';
      label.className = 'text-xs text-gray-400';
      label.textContent = 'SOMA';
    }
  }).catch(function() {
    var dot = document.getElementById('soma-dot');
    var label = document.getElementById('soma-label');
    dot.className = 'w-2 h-2 rounded-full bg-gray-300';
    label.className = 'text-xs text-gray-400';
    label.textContent = 'SOMA';
  });
}

// Initialize app
document.addEventListener('DOMContentLoaded', function() {
  navigate('contacts');
  checkSomaStatus();
  setInterval(checkSomaStatus, 10000);
});
