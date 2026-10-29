import React, { useContext, useMemo } from "react";
import styles from "./styles.module.css";
import Layout from "./Layouts";
import Header from "./Header";
import { CustomWidgetBuilderContext } from "..";
import HTMLEditor from "./CodeEditors/HTMLEditor";
import StyleEditor, { TitleControls } from "./CodeEditors/StyleEditor";
import JSEditor from "./CodeEditors/JSEditor";
import type { ContentProps } from "./CodeEditors/types";
import References from "./References";
import { AIAssistant, AIAssistantTitleControls } from "./AIAssistant";
import { useFeatureFlag } from "utils/hooks/useFeatureFlag";
import { FEATURE_FLAG } from "ee/entities/FeatureFlag";
import { CUSTOM_WIDGET_BUILDER_TABS } from "../constants";

const renderHTMLEditor = (props: ContentProps) => <HTMLEditor {...props} />;
const renderStyleEditor = (props: ContentProps) => <StyleEditor {...props} />;
const renderJSEditor = (props: ContentProps) => <JSEditor {...props} />;
const renderAIAssistant = (props: ContentProps) => <AIAssistant {...props} />;

export default function Editor() {
  const { isReferenceOpen } = useContext(CustomWidgetBuilderContext);
  const isAIEnabled = useFeatureFlag(
    FEATURE_FLAG.release_custom_widget_ai_builder,
  );

  const tabs = useMemo(() => {
    const defaultTabs = [
      {
        title: CUSTOM_WIDGET_BUILDER_TABS.HTML,
        children: renderHTMLEditor,
      },
      {
        title: CUSTOM_WIDGET_BUILDER_TABS.STYLE,
        titleControls: <TitleControls />,
        children: renderStyleEditor,
      },
      {
        title: CUSTOM_WIDGET_BUILDER_TABS.JS,
        children: renderJSEditor,
      },
    ];

    if (isAIEnabled) {
      defaultTabs.push({
        title: CUSTOM_WIDGET_BUILDER_TABS.AI,
        titleControls: <AIAssistantTitleControls />,
        children: renderAIAssistant,
      });
    }

    return defaultTabs;
  }, [isAIEnabled]);

  return (
    <div>
      <div className={styles.headerControls}>
        <Header />
      </div>
      <div className={styles.contentRightBody}>
        <Layout content={tabs} />
        {isReferenceOpen && <References />}
      </div>
    </div>
  );
}
