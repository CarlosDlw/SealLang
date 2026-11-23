use geometry::{MaybeAlias, Pair, PointAlias, Score};

func main() -> i32 {
    let point: PointAlias = PointAlias { x: 40, y: 2 };
    match point {
        PointAlias { x, y } => {
            let pair: Pair<Score> = (x, y);
            let maybe: MaybeAlias = MaybeAlias::Some(pair.0 + pair.1);
            match maybe {
                MaybeAlias::Some(total) => total,
                MaybeAlias::None => 0,
            }
        }
    }
}
