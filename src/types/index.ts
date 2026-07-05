export type ConnectionType = 'ticket' | 'endpoint_id';

export type LoadBalancingStrategy = 'round_robin' | 'random';

export interface NodeConfig {
  id: string;
  connectionType: ConnectionType;
  ticket: string;
  endpointId: string;
  domains: string[];
}

export interface ProxyConfig {
  nodes: NodeConfig[];
  domains: string[];
  localAddr: string;
  dnsAddr: string;
  upstreamDns: string;
  loadBalancing: LoadBalancingStrategy;
  useService: boolean;
}

export interface ProxyStatus {
  running: boolean;
  mode: string;
}

export interface ServiceStatus {
  installed: boolean;
  running: boolean;
}

export interface NodeInfo {
  id: string;
}
