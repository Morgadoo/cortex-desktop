import CATALOG from './connector-catalog.json';
import EXPERIMENTAL_CONNECTOR_PRESETS from './experimental-connector-presets.json';

/**
 * Public description of the connectors this app knows how to set up.
 *
 * Shared by the settings page and the connector bridge, so the web app sees the same
 * categories the settings page groups by. Holds display data only: anything needed to
 * authorize a connector (scopes, client ids, secrets, headers) stays in the settings page and
 * must never be added here, because the bridge hands this catalog to the Lumo web app.
 */

export type ConnectorCategoryId = 'device' | 'communication' | 'files' | 'work' | 'sales' | 'finance' | 'support' | 'recruiting' | 'observability' | 'data' | 'developer' | 'compliance' | 'research' | 'home' | 'custom';

export interface ConnectorCategory {
  id: ConnectorCategoryId;
  label: string;
}

export interface ConnectorCatalogEntry {
  /** Manifest id the connector registers under once added. */
  id: string;
  /** MCP server id, for entries backed by an MCP server. */
  server_id?: string;
  category: ConnectorCategoryId;
  icon: string;
  iconSrc: string;
  name: string;
  description: string;
  /** Endpoint of an HTTP MCP server. */
  url?: string;
}

export const CONNECTOR_CATEGORIES: ConnectorCategory[] = [
  { id: 'device', label: 'This device' },
  { id: 'communication', label: 'Communication' },
  { id: 'files', label: 'Files & storage' },
  { id: 'work', label: 'Work & projects' },
  { id: 'sales', label: 'Sales & CRM' },
  { id: 'finance', label: 'Finance' },
  { id: 'support', label: 'Support' },
  { id: 'recruiting', label: 'Recruiting' },
  { id: 'observability', label: 'Observability' },
  { id: 'data', label: 'Data & analytics' },
  { id: 'developer', label: 'Developer tools' },
  { id: 'compliance', label: 'Compliance' },
  { id: 'research', label: 'Research' },
  { id: 'home', label: 'Home & entertainment' },
  { id: 'custom', label: 'Custom' },
];

/**
 * The entries are data, in `connector-catalog.json`, because the Rust side reads them too
 * (`src-tauri/src/tools/connectors/mcp/catalog.rs` trusts the read-only hints of catalog servers).
 */
export const BUILTIN_CONNECTORS = CATALOG.builtin as ConnectorCatalogEntry[];

export const FEATURED_MCP_SERVERS = CATALOG.featured as ConnectorCatalogEntry[];

const CATALOG_BY_ID = new Map([...BUILTIN_CONNECTORS, ...FEATURED_MCP_SERVERS].map((entry) => [entry.id, entry]));

/** Connectors the user added themselves fall in `custom`. */
export const categoryOf = (id: string): ConnectorCategoryId => CATALOG_BY_ID.get(id)?.category ?? 'custom';

/** Settings tab value that opens the connectors tab on one connector. */
export const SETTINGS_CONNECTOR_TAB_PREFIX = 'connectors/';

/** Experimental presets stay hidden unless the frontend flag is enabled. */
export const getFeaturedMcpServers = (experimentalConnectors: boolean): ConnectorCatalogEntry[] => FEATURED_MCP_SERVERS.filter((entry) => experimentalConnectors || !EXPERIMENTAL_CONNECTOR_PRESETS.includes(entry.server_id ?? ''));
