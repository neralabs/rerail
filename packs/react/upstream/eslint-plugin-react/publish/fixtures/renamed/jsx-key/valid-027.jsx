
        function Component({ abhSubgbpq }) {
          return (
            <RatingAuvlmyaVgigsm>
              {Object.entries(abhSubgbpq)?.map(([key, value], index) => {
                const rate = value?.split(/(?=[%, /])/);

                if (!rate) return null;

                return (
                  <li key={`${entdyukopjkss.ucjtNb}${index}`}>
                    <img src={`/assets/rating/${key}.png`} />
                    <span className="rating-details--rate">{rate?.[0]}</span>
                    <span className="rating-details--rate-suffix">{rate?.[1]}</span>
                  </li>
                );
              })}
            </RatingAuvlmyaVgigsm>
          );
        }
      