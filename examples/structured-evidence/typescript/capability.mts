/** Project one immutable synthetic Deployment observation without live authority. */
export interface Capability {
  id: string; version: number; inputSchema: Record<string, unknown>; outputSchema: Record<string, unknown>;
  implementation: string; resources: readonly string[]; effectClass: 'read'; timeoutMs: number; outputBytes: number;
  credentialOwner: 'none'; reconciliation: string;
}

export function declareCapability(inputSchema: Record<string, unknown>, outputSchema: Record<string, unknown>): Capability {
  return Object.freeze({
    id: 'kubernetes_deployment_snapshot', version: 1, inputSchema, outputSchema,
    implementation: 'command:structured-deployment-fixture/1', resources: ['one-frozen-deployment'],
    effectClass: 'read', timeoutMs: 1000, outputBytes: 8192, credentialOwner: 'none',
    reconciliation: 'Reuse the original admitted invocation observation. A fresh collection needs a new operation identity and admission.',
  });
}

type ObjectValue = Record<string, unknown>;
const targetKeys = ['cluster_uid', 'namespace_uid', 'namespace', 'kind', 'name', 'uid', 'resource_version'] as const;

function require(condition: boolean): asserts condition {
  if (!condition) throw new Error('structured-evidence-rejected');
}

function object(value: unknown, keys: readonly string[]): ObjectValue {
  require(typeof value === 'object' && value !== null && !Array.isArray(value));
  const record = value as ObjectValue;
  require(Object.keys(record).length === keys.length && keys.every(key => Object.hasOwn(record, key)));
  return record;
}

function identifier(value: unknown, maximum: number): string {
  require(typeof value === 'string' && value.length <= maximum && /^[A-Za-z0-9][A-Za-z0-9._:/-]*$/.test(value));
  return value;
}

function name(value: unknown): string {
  require(typeof value === 'string' && /^[a-z0-9](?:[-a-z0-9]{0,61}[a-z0-9])?$/.test(value));
  return value;
}

function integer(value: unknown, minimum: number, maximum: number): number {
  require(typeof value === 'number' && Number.isSafeInteger(value) && value >= minimum && value <= maximum);
  return value;
}

export function collect(inputValue: unknown, fixtureValue: unknown, invocationValue: unknown) {
  const input = object(inputValue, ['operationId', 'target']);
  const operationId = identifier(input.operationId, 160);
  const invocation_id = identifier(invocationValue, 160);
  const target = object(input.target, targetKeys);
  const fixture = object(fixtureValue, ['version', 'scope', 'deployment']);
  require(fixture.version === 'structured-deployment-fixture/1');
  const scope = object(fixture.scope, ['cluster_uid', 'namespace_uid']);
  const deployment = object(fixture.deployment, ['apiVersion', 'kind', 'metadata', 'spec', 'status']);
  require(deployment.apiVersion === 'apps/v1' && deployment.kind === 'Deployment');
  const metadata = object(deployment.metadata, ['namespace', 'name', 'uid', 'resourceVersion', 'generation']);
  const spec = object(deployment.spec, ['replicas']);
  const status = object(deployment.status, ['observedGeneration', 'replicas', 'updatedReplicas', 'readyReplicas', 'availableReplicas', 'unavailableReplicas']);
  const expected = {
    cluster_uid: identifier(scope.cluster_uid, 128), namespace_uid: identifier(scope.namespace_uid, 128),
    namespace: name(metadata.namespace), kind: 'Deployment', name: name(metadata.name),
    uid: identifier(metadata.uid, 128), resource_version: identifier(metadata.resourceVersion, 128),
  };
  require(targetKeys.every(key => target[key] === expected[key]));
  const facts = {
    generation: integer(metadata.generation, 1, 2147483647),
    observed_generation: integer(status.observedGeneration, 0, 2147483647),
    desired_replicas: integer(spec.replicas, 0, 100), replicas: integer(status.replicas, 0, 100),
    updated_replicas: integer(status.updatedReplicas, 0, 100), ready_replicas: integer(status.readyReplicas, 0, 100),
    available_replicas: integer(status.availableReplicas, 0, 100), unavailable_replicas: integer(status.unavailableReplicas, 0, 100),
  };
  return {
    version: 'structured-deployment-observation/1', operationId, invocation_id, target: expected,
    source: 'synthetic-fixture/1', freshness: 'NOT_RUN', assessment: 'NOT_RUN',
    coverage: { scope: 'one-deployment-point', omissions: ['pods', 'images', 'events', 'logs', 'live-freshness'] }, facts,
  };
}
