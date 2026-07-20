export function h(tag, attributes = {}, ...children) {
  const element = document.createElement(tag);
  for (const [key, value] of Object.entries(attributes)) {
    if (key === 'class') element.className = value;
    else if (key.startsWith('on') && typeof value === 'function') element.addEventListener(key.slice(2).toLowerCase(), value);
    else if (key === 'checked' || key === 'disabled' || key === 'selected') element[key] = Boolean(value);
    else if (value !== undefined && value !== null) element.setAttribute(key, String(value));
  }
  for (const child of children.flat()) {
    if (child === null || child === undefined) continue;
    element.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return element;
}

export function formatList(values, formatter = String) {
  return values?.length ? values.map(formatter).join(', ') : '—';
}

export function interfaceRef(adapter) {
  return { guid: adapter.guid, macAddress: adapter.macAddress, description: adapter.description, alias: adapter.alias, interfaceIndex: adapter.interfaceIndex };
}
