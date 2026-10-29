import React, { useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { FormattedMessage, useIntl } from 'dumi';
import DumiSearchBar from 'dumi/theme-default/slots/SearchBar';
import classNames from 'classnames';
import { Col, Modal, Popover, Row, Select, Typography } from 'antd';
import { MenuOutlined, GithubOutlined } from '@ant-design/icons';
import type { DirectionType } from 'antd/es/config-provider';
import * as utils from '../../utils';
import { getThemeConfig, ping } from '../../utils';
import packageJson from '../../../../package.json';
import Logo from './Logo';
import More from './More';
import Navigation from './Navigation';
import Github from './Github';
import type { SiteContextProps } from '../SiteContext';
import SiteContext from '../SiteContext';
import { useLocation, useNavigate } from 'dumi';
import { ClassNames, css } from '@emotion/react';
import useSiteToken from '../../../hooks/useSiteToken';
import useLocale from '../../../hooks/useLocale';
import SwitchBtn from './SwitchBtn';

const RESPONSIVE_XS = 1120;
const RESPONSIVE_SM = 1200;

const { Option } = Select;

const antdVersion: string = packageJson.version;

const locales = {
  cn: {
    title: '🎉🎉🎉 Ant Design 5.0 发布！ 🎉🎉🎉',
    ok: '知道了',
  },
  en: {
    title: '🎉🎉🎉 Ant Design 5.0 is released! 🎉🎉🎉',
    ok: 'Got it',
  },
};

const useStyle = () => {
  const { token } = useSiteToken();
  const searchIconColor = '#ced4d9';

  return {
    header: css`
      position: relative;
      z-index: 10;
      max-width: 100%;
      background: ${token.colorBgContainer};
      box-shadow: ${token.boxShadow};

      @media only screen and (max-width: ${token.mobileMaxWidth}px) {
        text-align: center;
      }

      .nav-search-wrapper {
        display: flex;
        flex: auto;
      }

      .dumi-default-search-bar {
        border-inline-start: 1px solid rgba(0, 0, 0, 0.06);

        > svg {
          width: 14px;
          fill: ${searchIconColor};
        }

        > input {
          height: 22px;
          border: 0;

          &:focus {
            box-shadow: none;
          }

          &::placeholder {
            color: ${searchIconColor};
          }
        }

        .dumi-default-search-shortcut {
          color: ${searchIconColor};
          background-color: rgba(150, 150, 150, 0.06);
          border-color: rgba(100, 100, 100, 0.2);
          border-radius: 4px;
        }

        .dumi-default-search-popover {
          inset-inline-start: 11px;
          inset-inline-end: unset;

          &::before {
            inset-inline-start: 100px;
            inset-inline-end: unset;
          }
        }
      }
    `,
    menuRow: css`
      display: flex;
      align-items: center;
      margin: 0;

      > * {
        flex: none;
        margin: 0 12px 0 0;

        &:last-child {
          margin-right: 40px;
        }
      }

      ${token.antCls}-row-rtl & {
        > * {
          &:last-child {
            margin-right: 12px;
            margin-left: 40px;
          }
        }
      }
    `,
    popoverMenu: {
      width: 300,

      [`${token.antCls}-popover-inner-content`]: {
        padding: 0,
      },
    },
  };
};

export interface HeaderProps {
  changeDirection: (direction: DirectionType) => void;
}

let docsearch: any;
const triggerDocSearchImport = () => {
  if (docsearch) {
    return Promise.resolve();
  }

  // @ts-ignore
  return import('docsearch.js').then((ds) => {
    docsearch = ds.default;
  });
};

const V5_NOTIFICATION = 'antd@4.0.0-notification-sent';
const SHOULD_OPEN_ANT_DESIGN_MIRROR_MODAL = 'ANT_DESIGN_DO_NOT_OPEN_MIRROR_MODAL';

function disableAntdMirrorModal() {
  window.localStorage.setItem(SHOULD_OPEN_ANT_DESIGN_MIRROR_MODAL, 'true');
}

function shouldOpenAntdMirrorModal() {
  return !window.localStorage.getItem(SHOULD_OPEN_ANT_DESIGN_MIRROR_MODAL);
}

interface HeaderState {
  menuVisible: boolean;
  windowWidth: number;
  searching: boolean;
}

// ================================= Header =================================
const Header: React.FC<HeaderProps> = (props) => {
  const intl = useIntl();
  const { changeDirection } = props;
  const [isClient, setIsClient] = React.useState(false);
  const [locale, lang] = useLocale(locales);
  const { token } = useSiteToken();
  const [notify, setNotify] = React.useState<null | boolean>(null);

  // ========================= 发布通知 开始 =========================
  React.useEffect(() => {
    if (utils.isLocalStorageNameSupported()) {
      // 大版本发布后全局弹窗提示
      //   1. 点击『知道了』之后不再提示
      //   2. 超过截止日期后不再提示
      if (
        localStorage.getItem(V5_NOTIFICATION) !== 'true' &&
        Date.now() < new Date('2022/12/31').getTime()
      ) {
        setNotify(true);
        return;
      }
    }

    setNotify(false);
  }, []);

  function onClose() {
    setNotify(false);
    localStorage.setItem(V5_NOTIFICATION, 'true');
  }
  // ========================= 发布通知 结束 =========================

  const themeConfig = getThemeConfig();
  const [headerState, setHeaderState] = useState<HeaderState>({
    menuVisible: false,
    windowWidth: 1400,
    searching: false,
  });
  const { direction, isMobile } = useContext<SiteContextProps>(SiteContext);
  const pingTimer = useRef<NodeJS.Timeout | null>(null);
  const location = useLocation();
  const { pathname, search } = location;
  const navigate = useNavigate();

  const style = useStyle();

  const handleHideMenu = useCallback(() => {
    setHeaderState((prev) => ({ ...prev, menuVisible: false }));
  }, []);
  const onWindowResize = useCallback(() => {
    setHeaderState((prev) => ({ ...prev, windowWidth: window.innerWidth }));
  }, []);
  const onTriggerSearching = useCallback((searching: boolean) => {
    setHeaderState((prev) => ({ ...prev, searching }));
  }, []);
  const handleShowMenu = useCallback(() => {
    setHeaderState((prev) => ({ ...prev, menuVisible: true }));
  }, []);
  const onMenuVisibleChange = useCallback((visible: boolean) => {
    setHeaderState((prev) => ({ ...prev, menuVisible: visible }));
  }, []);
  const onDirectionChange = () => {
    changeDirection?.(direction !== 'rtl' ? 'rtl' : 'ltr');
  };

  useEffect(() => {
    handleHideMenu();
  }, [location]);

  useEffect(() => {
    setIsClient(typeof window !== 'undefined');
    onWindowResize();
    window.addEventListener('resize', onWindowResize);
    pingTimer.current = ping((status) => {
      if (status !== 'timeout' && status !== 'error') {
        if (
          // process.env.NODE_ENV === 'production' &&
          window.location.host !== 'ant-design.antgroup.com' &&
          shouldOpenAntdMirrorModal()
        ) {
          Modal.confirm({
            title: '提示',
            content: '内网用户推荐访问国内镜像以获得极速体验～',
            okText: '🚀 立刻前往',
            cancelText: '不再弹出',
            closable: true,
            zIndex: 99999,
            onOk() {
              window.open('https://ant-design.antgroup.com', '_self');
              disableAntdMirrorModal();
            },
            onCancel() {
              disableAntdMirrorModal();
            },
          });
        }
      }
    });
    return () => {
      window.removeEventListener('resize', onWindowResize);
      if (pingTimer.current) {
        clearTimeout(pingTimer.current);
      }
    };
  }, []);

  // eslint-disable-next-line class-methods-use-this
  const handleVersionChange = useCallback((url: string) => {
    const currentUrl = window.location.href;
    const currentPathname = window.location.pathname;
    if (/overview/.test(currentPathname) && /0?[1-39][0-3]?x/.test(url)) {
      window.location.href = currentUrl
        .replace(window.location.origin, url)
        .replace(/\/components\/overview/, `/docs${/0(9|10)x/.test(url) ? '' : '/react'}/introduce`)
        .replace(/\/$/, '');
      return;
    }
    window.location.href = currentUrl.replace(window.location.origin, url);
  }, []);

  const onLangChange = useCallback(() => {
    const currentProtocol = `${window.location.protocol}//`;
    const currentHref = window.location.href.slice(currentProtocol.length);

    if (utils.isLocalStorageNameSupported()) {
      localStorage.setItem('locale', utils.isZhCN(pathname) ? 'en-US' : 'zh-CN');
    }
    window.location.href =
      currentProtocol +
      currentHref.replace(
        window.location.pathname,
        utils.getLocalizedPathname(pathname, !utils.isZhCN(pathname), search).pathname,
      );
  }, [location]);

  const nextDirectionText = useMemo<string>(
    () => (direction !== 'rtl' ? 'RTL' : 'LTR'),
    [direction],
  );

  const getDropdownStyle = useMemo<React.CSSProperties>(
    () => (direction === 'rtl' ? { direction: 'ltr', textAlign: 'right' } : {}),
    [direction],
  );

  const { menuVisible, windowWidth, searching } = headerState;
  const docVersions: Record<string, string> = {
    [antdVersion]: antdVersion,
    ...themeConfig?.docVersions,
  };
  const versionOptions = Object.keys(docVersions).map((version) => (
    <Option value={docVersions[version]} key={version}>
      {version}
    </Option>
  ));

  const isHome = ['', 'index', 'index-cn'].includes(pathname);

  const isZhCN = lang === 'cn';
  const isRTL = direction === 'rtl';
  let responsive: null | 'narrow' | 'crowded' = null;
  if (windowWidth < RESPONSIVE_XS) {
    responsive = 'crowded';
  } else if (windowWidth < RESPONSIVE_SM) {
    responsive = 'narrow';
  }

  const headerClassName = classNames({
    clearfix: true,
    'home-header': isHome,
  });

  const sharedProps = {
    isZhCN,
    isRTL,
    isClient,
  };

  const navigationNode = (
    <Navigation
      key="nav"
      {...sharedProps}
      responsive={responsive}
      isMobile={isMobile}
      directionText={nextDirectionText}
      onLangChange={onLangChange}
      onDirectionChange={onDirectionChange}
    />
  );

  let menu: (React.ReactElement | null)[] = [
    navigationNode,
    <Popover
      open={!!notify}
      title={locale.title}
      content={
        <Typography style={{ marginTop: token.marginXS }}>
          {lang === 'cn' ? (
            <>
              <div>
                如果你发现任何新官网的问题，欢迎到{' '}
                <Typography.Link
                  target="_blank"
                  href="https://github.com/ant-design/ant-design/issues/38463"
                >
                  Github Issue
                </Typography.Link>{' '}
                反馈。
              </div>
              <div>如果你需要查看 v4 文档，请点击上侧切换。</div>
            </>
          ) : (
            <>
              <div>
                If you find any official site problem. Please feel free to report on{' '}
                <Typography.Link
                  target="_blank"
                  href="https://github.com/ant-design/ant-design/issues/38463"
                >
                  Github Issue
                </Typography.Link>
                .
              </div>
              <p>Click above Select to switch to v4 docs.</p>
            </>
          )}
        </Typography>
      }
    >
      <Select
        key="version"
        className="version"
        size="small"
        defaultValue={antdVersion}
        onChange={handleVersionChange}
        dropdownStyle={getDropdownStyle}
        getPopupContainer={(trigger) => trigger.parentNode}
        onClick={onClose}
      >
        {versionOptions}
      </Select>
    </Popover>,
    <More key="more" {...sharedProps} />,
    <SwitchBtn
      key="lang"
      onClick={onLangChange}
      value={utils.isZhCN(pathname) ? 1 : 2}
      label1="中"
      label2="En"
      tooltip1="中文 / English"
      tooltip2="English / 中文"
    />,
    <SwitchBtn
      key="direction"
      onClick={onDirectionChange}
      value={direction === 'rtl' ? 2 : 1}
      label1={
        <img src="https://mdn.alipayobjects.com/huamei_7uahnr/afts/img/A*6k0CTJA-HxUAAAAAAAAAAAAADrJ8AQ/original" />
      }
      tooltip1="LTR"
      label2={
        <img src="https://mdn.alipayobjects.com/huamei_7uahnr/afts/img/A*SZoaQqm2hwsAAAAAAAAAAAAADrJ8AQ/original" />
      }
      tooltip2="RTL"
      pure
    />,
    <a href="https://github.com/ant-design/ant-design" target="_blank" rel="noreferrer">
      <SwitchBtn
        key="github"
        value={1}
        label1={<GithubOutlined />}
        tooltip1="Github"
        label2={null}
        pure
      />
    </a>,
  ];

  if (windowWidth < RESPONSIVE_XS) {
    menu = searching ? [] : [navigationNode];
  } else if (windowWidth < RESPONSIVE_SM) {
    menu = searching ? [] : menu;
  }

  const colProps = isHome
    ? [{ flex: 'none' }, { flex: 'auto' }]
    : [
        { xxl: 4, xl: 5, lg: 6, md: 6, sm: 24, xs: 24 },
        { xxl: 20, xl: 19, lg: 18, md: 18, sm: 0, xs: 0 },
      ];

  return (
    <header css={style.header} className={headerClassName}>
      {isMobile && (
        <ClassNames>
          {({ css }) => (
            <Popover
              overlayClassName={css(style.popoverMenu)}
              placement="bottomRight"
              content={menu}
              trigger="click"
              open={menuVisible}
              arrowPointAtCenter
              onOpenChange={onMenuVisibleChange}
            >
              <MenuOutlined className="nav-phone-icon" onClick={handleShowMenu} />
            </Popover>
          )}
        </ClassNames>
      )}
      <Row style={{ flexFlow: 'nowrap', height: 64 }}>
        <Col {...colProps[0]}>
          <Logo {...sharedProps} location={location} />
        </Col>
        <Col {...colProps[1]} css={style.menuRow}>
          <div className="nav-search-wrapper">
            <DumiSearchBar />
          </div>
          {!isMobile && menu}
        </Col>
      </Row>
    </header>
  );
};

export default Header;
