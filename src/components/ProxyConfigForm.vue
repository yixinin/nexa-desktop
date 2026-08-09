<script setup lang="ts">
import { ref, watch, onMounted } from "vue";
import type { ProxyConfig, NodeConfig, ConnectionType, LoadBalancingStrategy } from "../types";
import { detectConnectionType } from "../utils/connection";

const props = defineProps<{
  config: ProxyConfig;
}>();

const emit = defineEmits<{
  (e: "update", config: ProxyConfig): void;
}>();

const localNodes = ref<NodeConfig[]>([]);
const domainsText = ref(props.config.domains.join("\n"));
const localAddr = ref(props.config.localAddr);
const dnsAddr = ref(props.config.dnsAddr);
const upstreamDns = ref(props.config.upstreamDns);
const loadBalancing = ref<LoadBalancingStrategy>(props.config.loadBalancing);
const useService = ref(props.config.useService);

const loadBalancingOptions: { value: LoadBalancingStrategy; label: string; desc: string }[] = [
  { value: 'round_robin', label: '轮询', desc: '按顺序依次分配到每个节点' },
  { value: 'random', label: '随机', desc: '随机选择一个节点' },
];

function updateConnectionType(node: NodeConfig) {
  const value = node.connectionType === 'ticket' ? node.ticket : node.endpointId;
  const info = detectConnectionType(value);
  node.connectionType = info.type;
  if (info.type === 'ticket') {
    node.ticket = info.value;
    node.endpointId = "";
  } else {
    node.endpointId = info.value;
    node.ticket = "";
  }
}

function getNodeLabel(index: number): string {
  return `节点 ${index + 1}`;
}

function getNodeTypeLabel(type: ConnectionType): string {
  return type === 'ticket' ? 'Ticket' : 'Endpoint ID';
}

function getNodeTypeColor(type: ConnectionType): string {
  return type === 'ticket' ? '#8b5cf6' : '#0ea5e9';
}

function addNode() {
  const newNode: NodeConfig = {
    id: `${Date.now()}-${Math.random().toString(36).substr(2, 9)}`,
    connectionType: 'ticket',
    ticket: "",
    endpointId: "",
    domains: [],
  };
  localNodes.value.push(newNode);
  handleUpdate();
}

function removeNode(index: number) {
  if (localNodes.value.length > 1) {
    localNodes.value.splice(index, 1);
    handleUpdate();
  }
}

function handleUpdate() {
  localNodes.value.forEach(updateConnectionType);
  
  const domains = domainsText.value
    .split("\n")
    .map(d => d.trim())
    .filter(d => d.length > 0);
  
  emit("update", {
    nodes: localNodes.value.map(n => ({ ...n })),
    domains,
    localAddr: localAddr.value,
    dnsAddr: dnsAddr.value,
    upstreamDns: upstreamDns.value,
    loadBalancing: loadBalancing.value,
    tunName: props.config.tunName,
    useService: useService.value,
  });
}

watch(() => props.config, (newConfig) => {
  localNodes.value = newConfig.nodes.length > 0 
    ? newConfig.nodes.map(n => ({ ...n, domains: n.domains || [] }))
    : [{
        id: `${Date.now()}-${Math.random().toString(36).substr(2, 9)}`,
        connectionType: 'ticket',
        ticket: "",
        endpointId: "",
        domains: [],
      }];
  domainsText.value = newConfig.domains.join("\n");
  localAddr.value = newConfig.localAddr;
  dnsAddr.value = newConfig.dnsAddr;
  upstreamDns.value = newConfig.upstreamDns;
  loadBalancing.value = newConfig.loadBalancing;
  useService.value = newConfig.useService;
}, { deep: true, immediate: true });

watch([domainsText, localAddr, dnsAddr, upstreamDns, loadBalancing, useService], handleUpdate);
watch(localNodes, handleUpdate, { deep: true });

onMounted(() => {
  if (localNodes.value.length === 0) {
    addNode();
  }
});
</script>

<template>
  <div class="config-form">
    <div class="card-header">
      <h2>代理配置</h2>
      <div class="card-header-decoration"></div>
    </div>
    
    <div class="form-group">
      <label class="form-label">
        <span class="label-text">节点列表</span>
        <span class="label-required">*</span>
        <button @click="addNode" class="add-node-btn">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <line x1="12" y1="5" x2="12" y2="19"/>
            <line x1="5" y1="12" x2="19" y2="12"/>
          </svg>
          添加节点
        </button>
      </label>
      
      <div class="nodes-container">
        <div 
          v-for="(node, index) in localNodes" 
          :key="node.id" 
          class="node-card"
        >
          <div class="node-header">
            <span class="node-label">{{ getNodeLabel(index) }}</span>
            <span 
              class="type-badge" 
              :style="{ backgroundColor: getNodeTypeColor(node.connectionType) + '15', color: getNodeTypeColor(node.connectionType), borderColor: getNodeTypeColor(node.connectionType) + '30' }"
            >
              <svg v-if="node.connectionType === 'ticket'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <rect x="3" y="3" width="18" height="18" rx="2" ry="2"/>
                <path d="M7 16V3h7v13"/>
                <path d="M17 16v-5a2 2 0 0 0-2-2H5"/>
              </svg>
              <svg v-else viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/>
              </svg>
              {{ getNodeTypeLabel(node.connectionType) }}
            </span>
            <button 
              v-if="localNodes.length > 1"
              @click="removeNode(index)" 
              class="remove-node-btn"
            >
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <line x1="18" y1="6" x2="6" y2="18"/>
                <line x1="6" y1="6" x2="18" y2="18"/>
              </svg>
            </button>
          </div>
          
          <div class="connection-input-wrapper">
            <div class="input-icon">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M10 13a5 5 0 0 1 5-5m0 0a5 5 0 0 1 5 5m-5-5v10"/>
              </svg>
            </div>
            <input
              v-model="node.connectionType === 'ticket' ? node.ticket : node.endpointId"
              type="text"
              :placeholder="node.connectionType === 'ticket' ? '输入 Ticket' : '输入 Endpoint ID'"
              class="form-input"
            />
          </div>
        </div>
      </div>
      <p class="hint">支持添加多个节点，当一个域名被配置到多个节点时，将使用负载均衡策略分发请求</p>
    </div>

    <div class="form-group">
      <label class="form-label">
        <span class="label-text">负载均衡策略</span>
      </label>
      <div class="load-balancing-options">
        <label 
          v-for="option in loadBalancingOptions" 
          :key="option.value"
          class="strategy-option"
          :class="{ active: loadBalancing === option.value }"
        >
          <input
            v-model="loadBalancing"
            :value="option.value"
            type="radio"
            class="strategy-radio"
          />
          <div class="strategy-content">
            <span class="strategy-label">{{ option.label }}</span>
            <span class="strategy-desc">{{ option.desc }}</span>
          </div>
        </label>
      </div>
    </div>

    <div class="form-group">
      <label for="domains" class="form-label">
        <span class="label-text">代理域名列表</span>
      </label>
      <div class="textarea-wrapper">
        <div class="input-icon">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
            <polyline points="14 2 14 8 20 8"/>
            <line x1="16" y1="13" x2="8" y2="13"/>
            <line x1="16" y1="17" x2="8" y2="17"/>
            <polyline points="10 9 9 9 8 9"/>
          </svg>
        </div>
        <textarea
          id="domains"
          v-model="domainsText"
          rows="4"
          placeholder="每行一个域名&#10;例如: example.com&#10;api.example.com"
          class="form-textarea"
        ></textarea>
      </div>
      <div class="domain-count">{{ domainsText.split('\n').filter(d => d.trim().length > 0).length }} 个域名</div>
    </div>

    <div class="form-row">
      <div class="form-group half">
        <label for="localAddr" class="form-label">
          <span class="label-text">本地代理地址</span>
        </label>
        <div class="input-wrapper">
          <div class="input-icon">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M12 2a10 10 0 0 0-10 10c0 4.42 2.87 8.17 6.84 9.49"/>
              <path d="M12 2a10 10 0 0 1 10 10c0 4.42-2.87 8.17-6.84 9.49"/>
              <path d="M12 12l4 4"/>
              <path d="M12 12l-4 4"/>
              <path d="M12 12l4-4"/>
              <path d="M12 12l-4-4"/>
            </svg>
          </div>
          <input
            id="localAddr"
            v-model="localAddr"
            type="text"
            placeholder="127.0.0.1:8080"
            class="form-input"
          />
        </div>
      </div>

      <div class="form-group half">
        <label for="dnsAddr" class="form-label">
          <span class="label-text">DNS 监听地址</span>
        </label>
        <div class="input-wrapper">
          <div class="input-icon">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <circle cx="12" cy="12" r="10"/>
              <polyline points="12 6 12 12 16 14"/>
            </svg>
          </div>
          <input
            id="dnsAddr"
            v-model="dnsAddr"
            type="text"
            placeholder="10.0.0.1:53"
            class="form-input"
          />
        </div>
      </div>
    </div>

    <div class="form-group">
      <label for="upstreamDns" class="form-label">
        <span class="label-text">上游 DNS</span>
      </label>
      <div class="input-wrapper">
        <div class="input-icon">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/>
          </svg>
        </div>
        <input
          id="upstreamDns"
          v-model="upstreamDns"
          type="text"
          placeholder="223.5.5.5:53"
          class="form-input"
        />
      </div>
    </div>

    <div class="form-group checkbox-group">
      <label class="checkbox-label">
        <div class="checkbox-wrapper">
          <input
            v-model="useService"
            type="checkbox"
            class="custom-checkbox"
          />
          <svg v-if="useService" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3">
            <polyline points="20 6 9 17 4 12"/>
          </svg>
        </div>
        <div class="checkbox-content">
          <span class="checkbox-text">使用服务模式运行</span>
          <span class="checkbox-hint">需要管理员权限，建议在生产环境使用</span>
        </div>
      </label>
    </div>
  </div>
</template>

<style scoped>
.config-form {
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

.form-group {
  margin-bottom: 20px;
}

.form-row {
  display: flex;
  gap: 20px;
}

.form-group.half {
  flex: 1;
}

.form-label {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 14px;
  font-weight: 500;
  color: var(--text-primary);
  margin-bottom: 8px;
}

.label-text {
  color: var(--text-primary);
}

.label-required {
  color: var(--error-500);
}

.add-node-btn {
  margin-left: auto;
  padding: 6px 12px;
  border: 1px dashed var(--primary-400);
  border-radius: var(--radius-sm);
  background: var(--primary-50);
  color: var(--primary-600);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  display: flex;
  align-items: center;
  gap: 4px;
  transition: all var(--transition-normal);
}

.add-node-btn:hover {
  background: var(--primary-100);
  border-style: solid;
}

.add-node-btn svg {
  width: 14px;
  height: 14px;
}

.nodes-container {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.node-card {
  background: var(--surface-2);
  border-radius: var(--radius-md);
  padding: 16px;
  border: 1px solid var(--border-color);
  transition: all var(--transition-normal);
}

.node-card:hover {
  border-color: var(--primary-300);
}

.node-header {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 12px;
}

.node-label {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-primary);
}

.type-badge {
  padding: 4px 10px;
  border-radius: 12px;
  font-size: 11px;
  font-weight: 500;
  border: 1px solid;
  display: flex;
  align-items: center;
  gap: 4px;
  transition: all var(--transition-normal);
}

.type-badge svg {
  width: 12px;
  height: 12px;
}

.remove-node-btn {
  margin-left: auto;
  width: 24px;
  height: 24px;
  border: none;
  background: transparent;
  border-radius: var(--radius-sm);
  color: var(--text-muted);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all var(--transition-fast);
}

.remove-node-btn:hover {
  background: var(--error-50);
  color: var(--error-500);
}

.remove-node-btn svg {
  width: 14px;
  height: 14px;
}

.input-wrapper,
.connection-input-wrapper,
.textarea-wrapper {
  position: relative;
}

.input-icon {
  position: absolute;
  left: 14px;
  top: 50%;
  transform: translateY(-50%);
  width: 18px;
  height: 18px;
  color: var(--text-muted);
  z-index: 1;
}

.input-icon svg {
  width: 100%;
  height: 100%;
}

.form-input,
.form-textarea {
  width: 100%;
  padding: 12px 14px;
  border: 1.5px solid var(--border-color);
  border-radius: var(--radius-md);
  font-size: 14px;
  transition: all var(--transition-normal);
  box-sizing: border-box;
  background: var(--surface-1);
  color: var(--text-primary);
}

.form-input {
  padding-left: 44px;
}

.form-input:focus,
.form-textarea:focus {
  outline: none;
  border-color: var(--primary-500);
  box-shadow: 0 0 0 3px rgba(59, 130, 246, 0.1);
}

.form-textarea {
  padding-left: 44px;
  resize: vertical;
  min-height: 100px;
  line-height: 1.6;
}

.hint {
  font-size: 12px;
  color: var(--text-muted);
  margin: 6px 0 0 0;
}

.domain-count {
  font-size: 12px;
  color: var(--text-muted);
  margin-top: 8px;
  text-align: right;
}

.load-balancing-options {
  display: flex;
  gap: 12px;
}

.strategy-option {
  flex: 1;
  padding: 14px 16px;
  border: 2px solid var(--border-color);
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all var(--transition-normal);
  background: var(--surface-1);
}

.strategy-option:hover {
  border-color: var(--primary-300);
}

.strategy-option.active {
  border-color: var(--primary-500);
  background: var(--primary-50);
}

.strategy-radio {
  display: none;
}

.strategy-content {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.strategy-label {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-primary);
}

.strategy-desc {
  font-size: 12px;
  color: var(--text-muted);
}

.checkbox-group {
  padding: 16px;
  background: var(--surface-2);
  border-radius: var(--radius-md);
  border: 1px dashed var(--border-color);
}

.checkbox-label {
  display: flex;
  align-items: flex-start;
  gap: 14px;
  cursor: pointer;
}

.checkbox-wrapper {
  position: relative;
  width: 22px;
  height: 22px;
  flex-shrink: 0;
  margin-top: 2px;
}

.custom-checkbox {
  position: absolute;
  width: 100%;
  height: 100%;
  opacity: 0;
  cursor: pointer;
  z-index: 2;
}

.checkbox-wrapper::before {
  content: "";
  position: absolute;
  top: 0;
  left: 0;
  width: 22px;
  height: 22px;
  border: 2px solid var(--border-color);
  border-radius: 6px;
  transition: all var(--transition-fast);
}

.checkbox-wrapper:hover::before {
  border-color: var(--primary-400);
}

.custom-checkbox:checked + svg {
  display: block;
}

.custom-checkbox:checked ~::before {
  background: var(--primary-500);
  border-color: var(--primary-500);
}

.checkbox-wrapper svg {
  display: none;
  position: absolute;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  width: 14px;
  height: 14px;
  color: white;
  z-index: 1;
}

.checkbox-content {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.checkbox-text {
  font-size: 14px;
  font-weight: 500;
  color: var(--text-primary);
}

.checkbox-hint {
  font-size: 12px;
  color: var(--text-muted);
}

@media (max-width: 600px) {
  .form-row {
    flex-direction: column;
  }
  
  .load-balancing-options {
    flex-direction: column;
  }
}
</style>
