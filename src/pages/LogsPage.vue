<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";

interface LogEntry {
  id: number;
  timestamp: string;
  level: "trace" | "debug" | "info" | "warn" | "error";
  message: string;
}

const logs = ref<LogEntry[]>([]);
const filterLevel = ref<string>("all");
const autoScroll = ref(true);
const logContainer = ref<HTMLElement | null>(null);
let logId = 0;

const mockLogs: LogEntry[] = [
  { id: ++logId, timestamp: "14:32:15.234", level: "info", message: "应用启动成功" },
  { id: ++logId, timestamp: "14:32:15.456", level: "debug", message: "加载配置文件: config.json" },
  { id: ++logId, timestamp: "14:32:15.789", level: "info", message: "初始化 Iroh 连接池" },
  { id: ++logId, timestamp: "14:32:16.123", level: "warn", message: "检测到 WinTUN 驱动未安装" },
  { id: ++logId, timestamp: "14:32:16.345", level: "info", message: "回退到本地代理模式" },
  { id: ++logId, timestamp: "14:32:17.567", level: "info", message: "DNS 服务启动: 10.0.0.1:53" },
  { id: ++logId, timestamp: "14:32:17.890", level: "info", message: "HTTP 代理启动: 127.0.0.1:8080" },
  { id: ++logId, timestamp: "14:32:18.123", level: "debug", message: "连接到远程节点: abc123..." },
  { id: ++logId, timestamp: "14:32:18.456", level: "info", message: "代理服务运行中" },
];

function getLevelColor(level: string) {
  switch (level) {
    case "trace":
      return "#8b5cf6";
    case "debug":
      return "#3b82f6";
    case "info":
      return "#22c55e";
    case "warn":
      return "#f59e0b";
    case "error":
      return "#ef4444";
    default:
      return "#6b7280";
  }
}

function getLevelLabel(level: string) {
  switch (level) {
    case "trace":
      return "TRACE";
    case "debug":
      return "DEBUG";
    case "info":
      return "INFO";
    case "warn":
      return "WARN";
    case "error":
      return "ERROR";
    default:
      return level.toUpperCase();
  }
}

function filterLogs() {
  if (filterLevel.value === "all") return logs.value;
  return logs.value.filter(log => log.level === filterLevel.value);
}

function scrollToBottom() {
  if (logContainer.value && autoScroll.value) {
    logContainer.value.scrollTop = logContainer.value.scrollHeight;
  }
}

function clearLogs() {
  logs.value = [];
}

function copyLogs() {
  const text = logs.value
    .map(log => `${log.timestamp} [${log.level.toUpperCase()}] ${log.message}`)
    .join("\n");
  navigator.clipboard.writeText(text);
}

let intervalId: ReturnType<typeof setInterval> | null = null;

onMounted(() => {
  logs.value = [...mockLogs];
  
  intervalId = setInterval(() => {
    const levels: LogEntry["level"][] = ["trace", "debug", "info", "warn", "error"];
    const messages = [
      "处理 DNS 查询: example.com",
      "转发请求到远程节点",
      "连接池连接数: 5",
      "DNS 缓存命中: api.example.com",
      "收到新的代理请求",
      "连接心跳检测",
      "清理过期连接",
    ];
    
    const now = new Date();
    const timestamp = `${String(now.getHours()).padStart(2, '0')}:${String(now.getMinutes()).padStart(2, '0')}:${String(now.getSeconds()).padStart(2, '0')}.${String(now.getMilliseconds()).padStart(3, '0')}`;
    
    const newLog: LogEntry = {
      id: ++logId,
      timestamp,
      level: levels[Math.floor(Math.random() * levels.length)],
      message: messages[Math.floor(Math.random() * messages.length)],
    };
    
    logs.value.push(newLog);
    
    if (logs.value.length > 100) {
      logs.value = logs.value.slice(-100);
    }
    
    scrollToBottom();
  }, 2000);
  
  scrollToBottom();
});

onUnmounted(() => {
  if (intervalId) {
    clearInterval(intervalId);
  }
});
</script>

<template>
  <div class="logs-page">
    <div class="logs-header">
      <div class="logs-filters">
        <select v-model="filterLevel" class="filter-select">
          <option value="all">全部级别</option>
          <option value="trace">Trace</option>
          <option value="debug">Debug</option>
          <option value="info">Info</option>
          <option value="warn">Warn</option>
          <option value="error">Error</option>
        </select>
        
        <label class="checkbox-label">
          <input type="checkbox" v-model="autoScroll" class="custom-checkbox" />
          <svg v-if="autoScroll" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <polyline points="20 6 9 17 4 12"/>
          </svg>
          <span>自动滚动</span>
        </label>
      </div>
      
      <div class="logs-actions">
        <button class="btn btn-secondary" @click="copyLogs">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <rect x="9" y="9" width="13" height="13" rx="2" ry="2"/>
            <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>
          </svg>
          复制日志
        </button>
        <button class="btn btn-outline" @click="clearLogs">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M3 6h18"/>
            <path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/>
            <path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/>
          </svg>
          清空日志
        </button>
      </div>
    </div>
    
    <div ref="logContainer" class="logs-container">
      <div v-if="logs.length === 0" class="empty-state">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
          <polyline points="14 2 14 8 20 8"/>
          <line x1="16" y1="13" x2="8" y2="13"/>
          <line x1="16" y1="17" x2="8" y2="17"/>
          <polyline points="10 9 9 9 8 9"/>
        </svg>
        <span>暂无日志</span>
      </div>
      
      <TransitionGroup name="log" tag="div" class="logs-list">
        <div 
          v-for="log in filterLogs()" 
          :key="log.id" 
          class="log-item"
          :class="log.level"
        >
          <span class="log-timestamp">{{ log.timestamp }}</span>
          <span class="log-level" :style="{ color: getLevelColor(log.level) }">
            {{ getLevelLabel(log.level) }}
          </span>
          <span class="log-message">{{ log.message }}</span>
        </div>
      </TransitionGroup>
    </div>
    
    <div class="logs-footer">
      <span class="logs-count">共 {{ logs.length }} 条日志</span>
    </div>
  </div>
</template>

<style scoped>
.logs-page {
  display: flex;
  flex-direction: column;
  height: calc(100vh - 180px);
  background: var(--surface-1);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-card);
  border: 1px solid var(--border-light);
  overflow: hidden;
}

.logs-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 16px 20px;
  border-bottom: 1px solid var(--border-light);
  background: var(--surface-2);
}

.logs-filters {
  display: flex;
  align-items: center;
  gap: 16px;
}

.filter-select {
  padding: 8px 12px;
  border: 1.5px solid var(--border-color);
  border-radius: var(--radius-sm);
  font-size: 13px;
  background: var(--surface-1);
  color: var(--text-primary);
  cursor: pointer;
  transition: all var(--transition-normal);
}

.filter-select:focus {
  outline: none;
  border-color: var(--primary-500);
}

.checkbox-label {
  display: flex;
  align-items: center;
  gap: 6px;
  cursor: pointer;
  font-size: 13px;
  color: var(--text-secondary);
}

.checkbox-label svg {
  width: 14px;
  height: 14px;
  color: var(--success-500);
}

.custom-checkbox {
  width: 16px;
  height: 16px;
  accent-color: var(--primary-500);
}

.logs-actions {
  display: flex;
  gap: 10px;
}

.btn {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 14px;
  border: none;
  border-radius: var(--radius-md);
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: all var(--transition-normal);
}

.btn svg {
  width: 16px;
  height: 16px;
}

.btn-secondary {
  background: var(--surface-3);
  color: var(--text-secondary);
  border: 1px solid var(--border-color);
}

.btn-secondary:hover {
  background: var(--surface-4);
}

.btn-outline {
  background: transparent;
  border: 1.5px solid var(--border-color);
  color: var(--text-secondary);
}

.btn-outline:hover {
  background: var(--surface-2);
  border-color: var(--error-400);
  color: var(--error-600);
}

.logs-container {
  flex: 1;
  overflow-y: auto;
  padding: 16px 20px;
  font-family: 'SF Mono', Monaco, 'Courier New', monospace;
  font-size: 13px;
  line-height: 1.6;
}

.logs-container::-webkit-scrollbar {
  width: 6px;
}

.logs-container::-webkit-scrollbar-track {
  background: var(--surface-2);
}

.logs-container::-webkit-scrollbar-thumb {
  background: var(--border-color);
  border-radius: 3px;
}

.logs-container::-webkit-scrollbar-thumb:hover {
  background: var(--text-muted);
}

.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: var(--text-muted);
  gap: 12px;
}

.empty-state svg {
  width: 48px;
  height: 48px;
  opacity: 0.5;
}

.logs-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.log-item {
  display: flex;
  gap: 12px;
  padding: 6px 10px;
  border-radius: var(--radius-sm);
  transition: background var(--transition-fast);
}

.log-item:hover {
  background: var(--surface-2);
}

.log-timestamp {
  color: var(--text-muted);
  flex-shrink: 0;
  min-width: 100px;
}

.log-level {
  font-weight: 600;
  flex-shrink: 0;
  min-width: 50px;
}

.log-message {
  color: var(--text-primary);
  word-break: break-all;
}

.log-item.error {
  background: var(--error-50);
}

.log-item.warn {
  background: var(--warning-50);
}

.logs-footer {
  padding: 12px 20px;
  border-top: 1px solid var(--border-light);
  background: var(--surface-2);
}

.logs-count {
  font-size: 12px;
  color: var(--text-muted);
}

.log-enter-active,
.log-leave-active {
  transition: all 0.3s ease;
}

.log-enter-from {
  opacity: 0;
  transform: translateY(-10px);
}

.log-leave-to {
  opacity: 0;
  transform: translateX(-20px);
}

@media (max-width: 600px) {
  .logs-header {
    flex-direction: column;
    gap: 12px;
    align-items: stretch;
  }
  
  .logs-filters {
    justify-content: space-between;
  }
  
  .logs-actions {
    justify-content: flex-end;
  }
  
  .log-item {
    flex-wrap: wrap;
  }
  
  .log-timestamp {
    min-width: auto;
  }
}
</style>