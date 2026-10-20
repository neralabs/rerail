
        import React, { FC, useRef, useState } from 'react';

        import './ResourceVideo.sass';
        import VimeoVideoPlayInModal from '../vimeoVideoPlayInModal/VimeoVideoPlayInModal';

        type Props = {
          videoUrl: string;
          videoTitle: string;
        };
        const ResourceVideo: FC<Props> = ({
          videoUrl,
          videoTitle,
        }: Props): JSX.Element => {
          return (
            <div className="resource-video">
              <VimeoVideoPlayInModal videoUrl={videoUrl} />
              <h3>{videoTitle}</h3>
            </div>
          );
        };

        export default ResourceVideo;
      
// features: [types], parser: @typescript-eslint/parser