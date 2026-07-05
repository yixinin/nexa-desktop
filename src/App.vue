<script setup lang="ts">
import { ref } from "vue";
import { useRouter, useRoute } from "vue-router";
import Toast from "./components/Toast.vue";
import ConfirmDialog from "./components/ConfirmDialog.vue";
import { useConfigStore } from "./composables/useConfigStore";

const router = useRouter();
const route = useRoute();

interface ToastItem {
  id: number;
  type: "success" | "error" | "warning" | "info";
  message: string;
  duration?: number;
}

const { proxyStatus } = useConfigStore();

const toasts = ref<ToastItem[]>([]);
const activeTooltip = ref<string | null>(null);

const confirmDialog = ref({
  visible: false,
  title: "",
  message: "",
  confirmText: "",
  cancelText: "",
  type: "info" as "warning" | "danger" | "info",
  onConfirm: () => {},
});

function removeToast(id: number) {
  toasts.value = toasts.value.filter(t => t.id !== id);
}

function handleConfirm() {
  confirmDialog.value.onConfirm();
  confirmDialog.value.visible = false;
}

function handleCancel() {
  confirmDialog.value.visible = false;
}

function showTooltip(id: string) {
  activeTooltip.value = id;
}

function hideTooltip() {
  activeTooltip.value = null;
}

const navItems = [
  { path: "/", label: "操作", icon: "activity", id: "nav-dashboard" },
  { path: "/config", label: "配置", icon: "settings", id: "nav-config" },
  { path: "/settings", label: "设置", icon: "gear", id: "nav-settings" },
  { path: "/logs", label: "日志", icon: "file-text", id: "nav-logs" },
];

function getNavIcon(icon: string) {
  switch (icon) {
    case "activity":
      return `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/></svg>`;
    case "settings":
      return `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>`;
    case "gear":
      return `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z"/></svg>`;
    case "file-text":
      return `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="16" y1="13" x2="8" y2="13"/><line x1="16" y1="17" x2="8" y2="17"/><polyline points="10 9 9 9 8 9"/></svg>`;
    default:
      return `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/></svg>`;
  }
}
</script>

<template>
  <div class="app-container">
    <aside class="app-sidebar">
      <div class="sidebar-top">
        <div class="logo-section">
          <div class="logo-icon">
            <svg viewBox="0 0 32 32" fill="none" xmlns="http://www.w3.org/2000/svg">
              <path d="M16 4L4 28h24L16 4z" fill="currentColor"/>
              <path d="M16 8L8 24h16L16 8z" fill="rgba(255,255,255,0.3)"/>
              <circle cx="16" cy="16" r="4" fill="white"/>
            </svg>
          </div>
        </div>
        
        <div class="status-indicator-sidebar" :class="{ running: proxyStatus.running }">
          <span class="status-dot"></span>
          <span>{{ proxyStatus.running ? '运行中' : '已停止' }}</span>
        </div>
      </div>

      <nav class="sidebar-nav">
        <button
          v-for="item in navItems"
          :key="item.path"
          class="nav-item"
          :class="{ active: route.path === item.path }"
          @click="router.push(item.path)"
          @mouseenter="showTooltip(item.id)"
          @mouseleave="hideTooltip"
        >
          <span class="nav-icon" v-html="getNavIcon(item.icon)"></span>
          <span v-if="route.path === item.path" class="nav-active-bg"></span>
        </button>
        
        <div 
          v-for="item in navItems"
          :key="item.path + '-tooltip'"
          class="nav-tooltip"
          :class="{ visible: activeTooltip === item.id && route.path !== item.path }"
        >
          {{ item.label }}
        </div>
      </nav>

      <div class="sidebar-bottom">
        <span class="version">v0.1.0</span>
      </div>
    </aside>

    <main class="app-main">
      <header class="page-header">
        <h1 class="page-title">
          {{ navItems.find(n => n.path === route.path)?.label || 'Pipe UI' }}
        </h1>
      </header>
      
      <div class="page-content">
        <router-view v-slot="{ Component }">
          <Transition name="page" mode="out-in">
            <component :is="Component" />
          </Transition>
        </router-view>
      </div>
    </main>

    <Toast :toasts="toasts" @remove="removeToast" />
    <ConfirmDialog
      :visible="confirmDialog.visible"
      :title="confirmDialog.title"
      :message="confirmDialog.message"
      :confirm-text="confirmDialog.confirmText"
      :cancel-text="confirmDialog.cancelText"
      :type="confirmDialog.type"
      @confirm="handleConfirm"
      @cancel="handleCancel"
    />
  </div>
</template>

<style>
:root {
  --primary-50: #eff6ff;
  --primary-100: #dbeafe;
  --primary-200: #bfdbfe;
  --primary-300: #93c5fd;
  --primary-400: #60a5fa;
  --primary-500: #3b82f6;
  --primary-600: #2563eb;
  --primary-700: #1d4ed8;
  --primary-800: #1e40af;
  --primary-900: #1e3a8a;
  
  --success-50: #f0fdf4;
  --success-100: #dcfce7;
  --success-500: #22c55e;
  --success-600: #16a34a;
  --success-700: #15803d;
  
  --warning-50: #fffbeb;
  --warning-100: #fef3c7;
  --warning-500: #f59e0b;
  --warning-600: #d97706;
  --warning-700: #b45309;
  
  --error-50: #fef2f2;
  --error-100: #fee2e2;
  --error-500: #ef4444;
  --error-600: #dc2626;
  --error-700: #b91c1c;
  
  --surface-1: #ffffff;
  --surface-2: #f8fafc;
  --surface-3: #f1f5f9;
  --surface-4: #e2e8f0;
  --surface-sidebar: #0f172a;
  --surface-sidebar-hover: #1e293b;
  
  --text-primary: #1e293b;
  --text-secondary: #64748b;
  --text-muted: #94a3b8;
  --text-sidebar: #94a3b8;
  --text-sidebar-active: #f1f5f9;
  
  --border-color: #e2e8f0;
  --border-light: #f1f5f9;
  --border-sidebar: #334155;
  
  --shadow-sm: 0 1px 2px 0 rgb(0 0 0 / 0.05);
  --shadow-md: 0 4px 6px -1px rgb(0 0 0 / 0.1), 0 2px 4px -2px rgb(0 0 0 / 0.1);
  --shadow-lg: 0 10px 15px -3px rgb(0 0 0 / 0.1), 0 4px 6px -4px rgb(0 0 0 / 0.1);
  --shadow-card: 0 4px 20px -2px rgba(59, 130, 246, 0.08);
  
  --radius-sm: 6px;
  --radius-md: 10px;
  --radius-lg: 16px;
  --radius-xl: 20px;
  
  --transition-fast: 150ms ease;
  --transition-normal: 250ms ease;
  --transition-slow: 350ms ease;
  
  --gradient-primary: linear-gradient(135deg, #3b82f6 0%, #8b5cf6 50%, #06b6d4 100%);
  --gradient-secondary: linear-gradient(135deg, #0f172a 0%, #1e293b 100%);
  --gradient-surface: linear-gradient(180deg, #f8fafc 0%, #f1f5f9 100%);
  
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
  font-size: 14px;
  line-height: 1.6;
  font-weight: 400;
  color: var(--text-primary);
  font-synthesis: none;
  text-rendering: optimizeLegibility;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}

* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

body {
  min-height: 100vh;
  background: var(--gradient-surface);
  overflow: hidden;
}

#app {
  height: 100vh;
}
</style>

<style scoped>
.app-container {
  height: 100vh;
  display: flex;
  overflow: hidden;
}

.app-sidebar {
  width: 60px;
  background: var(--surface-sidebar);
  display: flex;
  flex-direction: column;
  padding: 12px 0;
  flex-shrink: 0;
  border-right: 1px solid var(--border-sidebar);
}

.sidebar-top {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 16px;
  padding-bottom: 16px;
  border-bottom: 1px solid var(--border-sidebar);
}

.logo-section {
  padding: 8px;
}

.logo-icon {
  width: 36px;
  height: 36px;
  background: rgba(59, 130, 246, 0.2);
  border-radius: var(--radius-md);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--primary-400);
}

.logo-icon svg {
  width: 22px;
  height: 22px;
}

.status-indicator-sidebar {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  font-size: 10px;
  color: var(--text-sidebar);
}

.status-indicator-sidebar.running {
  color: var(--success-400);
}

.status-indicator-sidebar .status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--text-muted);
}

.status-indicator-sidebar.running .status-dot {
  background: var(--success-500);
  box-shadow: 0 0 8px var(--success-500);
  animation: pulse 2s infinite;
}

@keyframes pulse {
  0%, 100% {
    opacity: 1;
  }
  50% {
    opacity: 0.5;
  }
}

.sidebar-nav {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  padding-top: 16px;
  gap: 4px;
  position: relative;
}

.nav-item {
  width: 44px;
  height: 44px;
  background: transparent;
  border: none;
  border-radius: var(--radius-md);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  position: relative;
  transition: all var(--transition-normal);
  color: var(--text-sidebar);
}

.nav-item:hover {
  background: var(--surface-sidebar-hover);
  color: var(--text-sidebar-active);
}

.nav-item.active {
  color: var(--text-sidebar-active);
}

.nav-icon {
  width: 20px;
  height: 20px;
  z-index: 1;
}

.nav-icon svg {
  width: 100%;
  height: 100%;
}

.nav-active-bg {
  position: absolute;
  left: 0;
  top: 50%;
  transform: translateY(-50%);
  width: 3px;
  height: 24px;
  background: var(--gradient-primary);
  border-radius: 0 2px 2px 0;
}

.nav-tooltip {
  position: absolute;
  left: calc(100% + 8px);
  top: 50%;
  transform: translateY(-50%) translateX(-10px);
  background: var(--surface-sidebar-hover);
  padding: 6px 12px;
  border-radius: var(--radius-sm);
  font-size: 12px;
  font-weight: 500;
  color: var(--text-sidebar-active);
  white-space: nowrap;
  opacity: 0;
  pointer-events: none;
  transition: all var(--transition-normal);
  box-shadow: var(--shadow-lg);
  border: 1px solid var(--border-sidebar);
}

.nav-tooltip.visible {
  opacity: 1;
  transform: translateY(-50%) translateX(0);
}

.nav-tooltip::before {
  content: "";
  position: absolute;
  left: -4px;
  top: 50%;
  transform: translateY(-50%);
  border-width: 4px;
  border-style: solid;
  border-color: transparent var(--surface-sidebar-hover) transparent transparent;
}

.sidebar-bottom {
  padding-top: 12px;
  border-top: 1px solid var(--border-sidebar);
  display: flex;
  justify-content: center;
}

.version {
  font-size: 10px;
  color: var(--text-sidebar);
}

.app-main {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.page-header {
  padding: 20px 24px;
  background: var(--surface-1);
  border-bottom: 1px solid var(--border-light);
  flex-shrink: 0;
}

.page-title {
  font-size: 20px;
  font-weight: 700;
  color: var(--text-primary);
  margin: 0;
}

.page-content {
  flex: 1;
  padding: 24px;
  overflow: auto;
}

.page-content::-webkit-scrollbar {
  width: 6px;
  height: 6px;
}

.page-content::-webkit-scrollbar-track {
  background: transparent;
}

.page-content::-webkit-scrollbar-thumb {
  background: var(--border-color);
  border-radius: 3px;
}

.page-enter-active,
.page-leave-active {
  transition: all var(--transition-normal);
}

.page-enter-from {
  opacity: 0;
  transform: translateY(10px);
}

.page-leave-to {
  opacity: 0;
  transform: translateY(-10px);
}

@media (max-width: 600px) {
  .app-sidebar {
    width: 56px;
    padding: 8px 0;
  }
  
  .nav-item {
    width: 40px;
    height: 40px;
  }
  
  .page-header {
    padding: 16px;
  }
  
  .page-title {
    font-size: 18px;
  }
  
  .page-content {
    padding: 16px;
  }
}
</style>