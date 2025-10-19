export interface JsonRpcRequest<P = unknown> {
  jsonrpc: '2.0';
  id: string;
  method: string;
  params?: P;
}

export interface JsonRpcSuccess<R = unknown> {
  jsonrpc: '2.0';
  id: string;
  result: R;
}

export interface JsonRpcError {
  code: number;
  message: string;
  data?: unknown;
}

export interface JsonRpcFailure {
  jsonrpc: '2.0';
  id: string;
  error: JsonRpcError;
}

export type JsonRpcResponse<R = unknown> = JsonRpcSuccess<R> | JsonRpcFailure;

export class RpcError extends Error {
  public readonly code: number;
  public readonly data?: unknown;

  constructor(message: string, code: number, data?: unknown) {
    super(message);
    this.name = 'RpcError';
    this.code = code;
    this.data = data;
  }
}

export class RpcClient {
  private readonly endpoint: string;
  private token?: string;
  private apiKey?: string;

  constructor(endpoint: string, token?: string, apiKey?: string) {
    this.endpoint = endpoint;
    this.token = token;
    this.apiKey = apiKey;
  }

  setBearerToken(token: string | undefined) {
    this.token = token;
  }

  setApiKey(apiKey: string | undefined) {
    this.apiKey = apiKey;
  }

  async call<P, R>(method: string, params?: P): Promise<R> {
    const request: JsonRpcRequest<P> = {
      jsonrpc: '2.0',
      id: crypto.randomUUID(),
      method,
      params
    };

    const headers: HeadersInit = {
      'Content-Type': 'application/json'
    };
    if (this.token) {
      headers['Authorization'] = `Bearer ${this.token}`;
    }
    if (this.apiKey) {
      headers['X-API-Key'] = this.apiKey;
    }

    const response = await fetch(this.endpoint, {
      method: 'POST',
      headers,
      body: JSON.stringify(request)
    });

    if (!response.ok) {
      throw new RpcError(`RPC transport failure: ${response.statusText}`, response.status);
    }

    const payload = (await response.json()) as JsonRpcResponse<R>;
    if ('error' in payload) {
      throw new RpcError(payload.error.message, payload.error.code, payload.error.data);
    }

    return payload.result;
  }
}
