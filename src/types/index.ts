export type ConnectionType = 'ticket' | 'endpoint_id';

export type RelayMode = 'pinned' | 'default' | 'disabled' | 'custom';

export type LoadBalancingStrategy = 'round_robin' | 'random';

export type TwoFactorAlgorithm = 'sha1' | 'sha256' | 'sha512';

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
  tunName: string;
  useService: boolean;
  relayMode: RelayMode;
  relayUrl: string;
  forceRelay: boolean;
  twoFactorEnabled: boolean;
  twoFactorClientId: string;
  twoFactorSecret: string;
  twoFactorAlgorithm: TwoFactorAlgorithm;
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
