<script setup lang="ts">
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

const isServiceRunning = ref(false);
const isLoading = ref(false);
const message = ref("");
const messageType = ref<"success" | "error" | "info">("info");

async function updateServiceStatus() {
  try {
    isServiceRunning.value = await invoke<boolean>("is_service_running");
  } catch (e) {
    console.error("Failed to check service status:", e);
  }
}

async function installService() {
  if (isLoading.value) return;
  
  isLoading.value = true;
  
  try {
    const result = await invoke<string>("install_service");
    message.value = result;
    messageType.value = "success";
    await updateServiceStatus();
  } catch (e: any) {
    message.value = e.message || "安装服务失败，需要管理员权限";
    messageType.value = "error";
    console.error("Failed to install service:", e);
  } finally {
    isLoading.value = false;
    setTimeout(() => {
      message.value = "";
    }, 3000);
  }
}

async function uninstallService() {
  if (isLoading.value) return;
  
  isLoading.value = true;
  
  try {
    const result = await invoke<string>("uninstall_service");
    message.value = result;
    messageType.value = "success";
    await updateServiceStatus();
  } catch (e: any) {
    message.value = e.message || "卸载服务失败，需要管理员权限";
    messageType.value = "error";
    console.error("Failed to uninstall service:", e);
  } finally {
    isLoading.value = false;
    setTimeout(() => {
      message.value = "";
    }, 3000);
  }
}

updateServiceStatus();
</script>

<template>
  <div class="service-manager">
    <div class="card-header">
      <h2>服务管理</h2>
      <div class="card-header-decoration"></div>
    </div>
    
    <div class="service-status">
      <div class="status-row">
        <div class="status-left">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/>
          </svg>
          <span class="status-label">服务状态</span>
        </div>
        <div class="status-right">
          <span class="status-value" :class="isServiceRunning ? 'running' : 'stopped'">
            <span class="status-dot"></span>
            {{ isServiceRunning ? '运行中' : '未运行' }}
          </span>
        </div>
      </div>
    </div>

    <div v-if="message" class="message" :class="messageType">
      <svg v-if="messageType === 'success'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <polyline points="20 6 9 17 4 12"/>
      </svg>
      <svg v-else-if="messageType === 'error'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <circle cx="12" cy="12" r="10"/>
        <line x1="12" y1="8" x2="12" y2="12"/>
        <line x1="12" y1="16" x2="12.01" y2="16"/>
      </svg>
      <svg v-else viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <circle cx="12" cy="12" r="10"/>
        <line x1="12" y1="16" x2="12" y2="12"/>
        <line x1="12" y1="8" x2="12.01" y2="8"/>
      </svg>
      <span>{{ message }}</span>
    </div>

    <div class="service-actions">
      <button
        class="btn btn-outline"
        :disabled="isLoading"
        @click="installService"
      >
        <svg v-if="isLoading" class="btn-icon spinner" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="12" cy="12" r="10"/>
        </svg>
        <svg v-else viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M12 5v14"/>
          <path d="M5 12h14"/>
        </svg>
        <span>{{ isLoading ? '安装中...' : '安装服务' }}</span>
      </button>
      
      <button
        class="btn btn-outline btn-danger"
        :disabled="isLoading"
        @click="uninstallService"
      >
        <svg v-if="isLoading" class="btn-icon spinner" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="12" cy="12" r="10"/>
        </svg>
        <svg v-else viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <line x1="18" y1="6" x2="6" y2="18"/>
          <line x1="6" y1="6" x2="18" y2="18"/>
        </svg>
        <span>{{ isLoading ? '卸载中...' : '卸载服务' }}</span>
      </button>
    </div>

    <div class="service-hints">
      <div class="hint-item">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"/>
        </svg>
        <span>服务模式需要管理员权限</span>
      </div>
      <div class="hint-item">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M12 8v4l3 3"/>
          <circle cx="12" cy="12" r="10"/>
        </svg>
        <span>服务会在系统启动时自动运行</span>
      </div>
      <div class="hint-item">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"/>
          <polyline points="22 4 12 14.01 9 11.01"/>
        </svg>
        <span>建议在生产环境使用服务模式</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.service-manager {
  background: var(--surface-1);
  border-radius: var(--radius-lg);
  padding: 28px;
  box-shadow: var(--shadow-card);
  border: 1px solid var(--border-light);
}

.card-header {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 24px;
}

.card-header h2 {
  margin: 0;
  font-size: 18px;
  font-weight: 600;
  color: var(--text-primary);
}

.card-header-decoration {
  flex: 1;
  height: 3px;
  background: var(--gradient-primary);
  border-radius: 2px;
}

.service-status {
  background: var(--surface-2);
  border-radius: var(--radius-md);
  padding: 18px 20px;
  margin-bottom: 16px;
  border: 1px solid var(--border-light);
}

.status-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.status-left {
  display: flex;
  align-items: center;
  gap: 10px;
}

.status-left svg {
  width: 18px;
  height: 18px;
  color: var(--text-muted);
}

.status-label {
  font-size: 14px;
  color: var(--text-secondary);
  font-weight: 500;
}

.status-right {
  display: flex;
  align-items: center;
}

.status-value {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 14px;
  font-weight: 600;
  padding: 6px 14px;
  border-radius: 14px;
  transition: all var(--transition-normal);
}

.status-value.running {
  color: var(--success-600);
  background: var(--success-50);
  border: 1px solid var(--success-200);
}

.status-value.stopped {
  color: var(--text-muted);
  background: var(--surface-3);
  border: 1px solid var(--border-color);
}

.status-dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
}

.status-value.running .status-dot {
  background: var(--success-500);
  animation: pulse 2s ease-in-out infinite;
}

.status-value.stopped .status-dot {
  background: var(--text-muted);
}

@keyframes pulse {
  0%, 100% {
    opacity: 1;
  }
  50% {
    opacity: 0.5;
  }
}

.message {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 14px;
  border-radius: var(--radius-md);
  margin-bottom: 16px;
  font-size: 13px;
  transition: all var(--transition-normal);
}

.message svg {
  width: 18px;
  height: 18px;
  flex-shrink: 0;
}

.message.success {
  background: var(--success-50);
  color: var(--success-700);
  border-left: 3px solid var(--success-500);
}

.message.error {
  background: var(--error-50);
  color: var(--error-700);
  border-left: 3px solid var(--error-500);
}

.message.info {
  background: var(--primary-50);
  color: var(--primary-700);
  border-left: 3px solid var(--primary-500);
}

.service-actions {
  display: flex;
  gap: 12px;
  margin-bottom: 20px;
  flex-wrap: wrap;
}

.btn {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 12px 20px;
  border-radius: var(--radius-md);
  font-size: 14px;
  font-weight: 500;
  cursor: pointer;
  transition: all var(--transition-normal);
}

.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
  transform: none !important;
}

.btn svg {
  width: 16px;
  height: 16px;
}

.btn-outline {
  background: transparent;
  border: 1.5px solid var(--border-color);
  color: var(--text-secondary);
}

.btn-outline:hover:not(:disabled) {
  background: var(--surface-2);
  border-color: var(--primary-400);
  color: var(--primary-600);
}

.btn-outline.btn-danger {
  border-color: var(--error-300);
  color: var(--error-600);
}

.btn-outline.btn-danger:hover:not(:disabled) {
  background: var(--error-50);
  border-color: var(--error-500);
}

.spinner {
  animation: spin 1s linear infinite;
}

@keyframes spin {
  from {
    transform: rotate(0deg);
  }
  to {
    transform: rotate(360deg);
  }
}

.service-hints {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.hint-item {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 10px 12px;
  background: var(--surface-2);
  border-radius: var(--radius-sm);
}

.hint-item svg {
  width: 14px;
  height: 14px;
  color: var(--text-muted);
  flex-shrink: 0;
  margin-top: 2px;
}

.hint-item span {
  font-size: 12px;
  color: var(--text-muted);
  line-height: 1.4;
}

@media (max-width: 480px) {
  .service-actions {
    flex-direction: column;
  }
  
  .btn {
    width: 100%;
    justify-content: center;
  }
}
</style>