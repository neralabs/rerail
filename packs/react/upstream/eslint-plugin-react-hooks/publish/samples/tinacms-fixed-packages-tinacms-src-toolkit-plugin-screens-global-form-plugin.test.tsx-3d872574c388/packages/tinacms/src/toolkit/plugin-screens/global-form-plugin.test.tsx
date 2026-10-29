import { render, screen } from '@testing-library/react';
import * as React from 'react';
import { describe, expect, it, vi, beforeEach } from 'vitest';
import {
  tinaReducer,
  initialState,
  type TinaAction,
} from '@toolkit/tina-state';

// Capture FormBuilder's props to assert it is never handed an undefined form
// (the original crash) and ultimately receives the restored global form.
const formBuilderSpy = vi.fn();
vi.mock('@toolkit/form-builder', () => ({
  FormBuilder: (props: any) => {
    formBuilderSpy(props);
    return <div data-testid='form-builder'>{props.form?.tinaForm?.id}</div>;
  },
}));

// Mirror the real provider: a stable cms instance whose `state` updates as the
// reducer runs, handed down through context. The harness publishes it as
// `currentCms` after each commit so tests can assert on it.
const CmsContext = React.createContext<unknown>(undefined);
let currentCms: any;
vi.mock('@toolkit/react-tinacms', () => ({
  useCMS: () => React.useContext(CmsContext),
}));

import { GlobalFormPlugin } from './index';

const makeForm = (id: string) =>
  ({
    id,
    label: 'Global Configuration',
    global: true,
    // Used by calculateBreadcrumbs when the active form id is set.
    getActiveField: () => ({ label: 'Global Configuration', namespace: [] }),
  }) as any;

// Renders the GlobalFormPlugin's Component against a real reducer. `seed`
// lets a test pre-populate state.forms to simulate the form already present.
function Harness({
  form,
  seed,
}: {
  form: any;
  seed?: TinaAction[];
}) {
  const [state, dispatch] = React.useReducer(tinaReducer, undefined, () => {
    let s = initialState({} as any);
    (seed || []).forEach((action) => {
      s = tinaReducer(s, action);
    });
    return s;
  });

  const cmsRef = React.useRef<any>({});
  cmsRef.current.state = state;
  cmsRef.current.dispatch = dispatch;
  React.useEffect(() => {
    currentCms = cmsRef.current;
  });

  const plugin = React.useMemo(() => new GlobalFormPlugin(form), [form]);
  const Comp = plugin.Component;
  return (
    <CmsContext.Provider value={cmsRef.current}>
      <Comp />
    </CmsContext.Provider>
  );
}

describe('GlobalFormPlugin', () => {
  beforeEach(() => {
    formBuilderSpy.mockClear();
    currentCms = undefined;
  });

  it('re-establishes the global form when it is missing from cms.state.forms', async () => {
    const form = makeForm('global-1');
    // No seed: state.forms is empty, reproducing the post-navigation-cleanup
    // state where the screen plugin is still registered but the form is gone.
    render(<Harness form={form} />);

    // The form is restored and FormBuilder eventually renders it.
    await screen.findByTestId('form-builder');

    // FormBuilder must never be called with an undefined form/tinaForm.
    expect(formBuilderSpy).toHaveBeenCalled();
    formBuilderSpy.mock.calls.forEach(([props]) => {
      expect(props.form).toBeTruthy();
      expect(props.form.tinaForm).toBeTruthy();
    });

    // The restored form is the plugin's stored form, active id is set, and
    // there are no duplicates.
    expect(currentCms.state.forms).toHaveLength(1);
    expect(currentCms.state.forms[0].tinaForm.id).toBe('global-1');
    expect(currentCms.state.activeFormId).toBe('global-1');
  });

  it('renders the existing form without duplicating it when already present', async () => {
    const form = makeForm('global-2');
    render(<Harness form={form} seed={[{ type: 'forms:add', value: form }]} />);

    await screen.findByTestId('form-builder');

    // No duplicate form was added.
    expect(currentCms.state.forms).toHaveLength(1);
    expect(currentCms.state.forms[0].tinaForm.id).toBe('global-2');
    // FormBuilder always received the present form, never undefined.
    formBuilderSpy.mock.calls.forEach(([props]) => {
      expect(props.form?.tinaForm?.id).toBe('global-2');
    });
  });
});
