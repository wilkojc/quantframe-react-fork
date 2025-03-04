use crate::enums::OrderMode;
use crate::error;
use crate::live_scraper::client::LiveScraperClient;
use crate::structs::Order;
use crate::{
    error::AppError,
    helper::{self, ColumnType, ColumnValue, ColumnValues},
    logger,
};
use eyre::eyre;
use polars::prelude::*;
use serde_json::json;
use std::collections::HashSet;
use std::vec;

pub struct ItemModule<'a> {
    pub client: &'a LiveScraperClient,
}

impl<'a> ItemModule<'a> {
    pub async fn check_stock(&self) -> Result<(), AppError> {
        logger::info_con("ItemModule", "Run item module");
        let db = self.client.db.lock()?.clone();

        let settings = self.client.settings.lock()?.clone().live_scraper;
        let order_mode = settings.stock_item.order_mode.clone();

        let wfm = self.client.wfm.lock()?.clone();

        // List of strings that will be checked
        let mut stock_items: Vec<String> = vec![];
        let mut stock_items_df = DataFrame::new(vec![
            Series::new("item_url", &[] as &[&str]),
            Series::new("owned", &[] as &[i32]),
        ])
        .unwrap();
        let mut popular_items: Vec<String> = vec![];
        let popular_items_df = self.get_buy_sell_overlap().await?;
        let whitelist_items: Vec<String> = settings.stock_item.whitelist.clone();

        // Get current orders from Warframe Market Sell and Buy orders.
        let (mut current_buy_orders_df, current_sell_orders_df) =
            wfm.orders().get_orders_as_dataframe().await?;

        // Delete orders base on order_mode
        let orders = wfm.orders().get_my_orders().await?;
        if order_mode == OrderMode::Buy {
            let mut current_index = 0;
            let total = orders.sell_orders.len();
            self.client.send_message("item.deleting_orders", Some(json!({ "count": 0, "total": total})));
            for order in orders.sell_orders {
                current_index += 1;
                self.client.send_message("item.deleting_orders", Some(json!({ "count": current_index, "total": total})));
                wfm.orders()
                    .delete(&order.id, "None", "None", "Any")
                    .await?;
            }
        } else if order_mode == OrderMode::Sell {
            let mut current_index = 0;
            let total = orders.buy_orders.len();
            self.client.send_message("item.deleting_orders", Some(json!({ "count": 0, "total": total})));
            for order in orders.buy_orders {
                current_index += 1;
                self.client.send_message("item.deleting_orders", Some(json!({ "count": current_index, "total": total})));
                wfm.orders()
                    .delete(&order.id, "None", "None", "Any")
                    .await?;
            }
        }

        // Get the items names from the database based on order_mode
        if order_mode == OrderMode::Sell || order_mode == OrderMode::Both {
            stock_items_df = db
                .stock_item()
                .convet_stock_item_to_datafream(db.stock_item().get_items().await?)
                .unwrap();
            stock_items.append(&mut db.stock_item().get_items_names().await?.clone());
        }

        // Get the items names from the database based on order_mode
        if order_mode == OrderMode::Buy || order_mode == OrderMode::Both {
            let mut items: Vec<String> = match helper::get_column_values(
                popular_items_df.clone(),
                None,
                "name",
                ColumnType::String,
            )? {
                ColumnValues::String(values) => values,
                _ => return Err(AppError::new("LiveScraper", eyre!("Expected f64 values"))),
            };

            popular_items.append(&mut items);

            if current_buy_orders_df.shape().0 != 0 {
                current_buy_orders_df = current_buy_orders_df
                    .lazy()
                    .filter(
                        col("url_name")
                            .is_in(lit(Series::new("interesting_items", popular_items.clone()))),
                    )
                    .collect()
                    .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;

                let order_buy_df = helper::filter_and_extract(
                    popular_items_df.clone(),
                    None,
                    vec!["name", "closedAvg"],
                )?;

                current_buy_orders_df = current_buy_orders_df
                    .inner_join(&order_buy_df, ["url_name"], ["name"])
                    .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;

                current_buy_orders_df = current_buy_orders_df
                    .clone()
                    .lazy()
                    .fill_nan(lit(0.0).alias("closedAvg"))
                    .fill_nan(lit(0.0).alias("platinum"))
                    .with_column((col("closedAvg") - col("platinum")).alias("potential_profit"))
                    .collect()
                    .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;
            }
        }

        // Combine inventory_names and interesting_items and whitelist
        let all_interesting_items = stock_items
            .clone()
            .into_iter()
            .chain(popular_items.clone().into_iter())
            .chain(whitelist_items.clone().into_iter())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        // Remove duplicates
        let all_interesting_items: HashSet<String> = HashSet::from_iter(all_interesting_items);

        logger::info_file(
            "LiveScraper",
            format!("Interesting items: {:?}", all_interesting_items).as_str(),
            Some(self.client.log_file.as_str()),
        );

        let mut current_index = all_interesting_items.len();
        // Loop through all interesting items
        for item in all_interesting_items.clone() {
            if self.client.is_running() == false || item == "" {
                continue;
            }
            current_index -= 1;
            
            logger::info_con(
                "LiveScraper",
                format!(
                    "Checking item: {}, ({}/{})",
                    item,
                    current_index,
                    all_interesting_items.len()
                )
                .as_str(),
            );
            self.client.send_message("item.checking", Some(json!({ "name": item, "count": current_index, "total": all_interesting_items.len()})));

            let item_live_orders_df = wfm.orders().get_ordres_by_item(&item).await?;
            // Check if item_orders_df is empty and skip if it is
            if item_live_orders_df.height() == 0 {
                continue;
            }
            let item_stats = popular_items_df
                .clone()
                .lazy()
                .filter(col("name").eq(lit(item.clone())))
                .collect()
                .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;
            // Check if item is in all_interesting_items
            if !popular_items.contains(&item) {
                logger::info_file(
                    "LiveScraper",
                    format!("Item: {item} is not in all_interesting_items").as_str(),
                    Some(self.client.log_file.as_str()),
                );
                let item_info = wfm.items().get_item(item.to_string()).await?;

                let item_id = item_info.id;
                let item_rank = item_info.items_in_set.get(0).unwrap().mod_max_rank;
                self.compare_live_orders_when_selling(
                    &item,
                    &item_id,
                    item_rank,
                    current_sell_orders_df.clone(),
                    &item_live_orders_df,
                    &item_stats,
                    &stock_items_df,
                )
                .await?;
                continue;
            }

            // Get the item_id and item_rank
            let item_id: String = match helper::get_column_value(
                popular_items_df.clone(),
                Some(col("name").eq(lit(item.clone()))),
                "item_id",
                ColumnType::String,
            )? {
                ColumnValue::String(values) => values.unwrap_or("".to_string()),
                _ => return Err(AppError::new("LiveScraper", eyre!("Expected f64 values"))),
            };

            let item_rank: Option<f64> = match helper::get_column_value(
                popular_items_df.clone(),
                Some(col("name").eq(lit(item.clone()))),
                "mod_rank",
                ColumnType::F64,
            )? {
                ColumnValue::F64(values) => values,
                _ => return Err(AppError::new("LiveScraper", eyre!("Expected f64 values"))),
            };

            let item_stats = popular_items_df
                .clone()
                .lazy()
                .filter(col("name").eq(lit(item.clone())))
                .collect()
                .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;

            if order_mode == OrderMode::Buy || order_mode == OrderMode::Both {
                self.compare_live_orders_when_buying(
                    &item,
                    &item_id,
                    item_rank,
                    current_buy_orders_df.clone(),
                    &item_live_orders_df,
                    &item_stats,
                    &stock_items_df,
                )
                .await?;
            }

            if order_mode == OrderMode::Sell || order_mode == OrderMode::Both {
                self.compare_live_orders_when_selling(
                    &item,
                    &item_id,
                    item_rank,
                    current_sell_orders_df.clone(),
                    &item_live_orders_df,
                    &item_stats,
                    &stock_items_df,
                )
                .await?;
            }
        }
        Ok(())
    }
    fn get_week_increase(&self, df: &DataFrame, row_name: &str) -> Result<f64, AppError> {
        // Pre-filter DataFrame based on "order_type" == "closed"
        let week_df = df
            .clone()
            .lazy()
            .filter(
                col("order_type")
                    .eq(lit("closed"))
                    .and(col("name").eq(lit(row_name))),
            )
            .collect()
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;

        // Sort the DataFrame by "datetime" column
        let week_df = helper::sort_dataframe(week_df, "datetime", true)?;

        // Assuming the filtered DataFrame has at least 7 rows
        if week_df.height() >= 7 {
            let avg_price_series = week_df
                .column("median")
                .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;
            let avg_price_array = avg_price_series
                .f64()
                .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;
            let first_avg_price = avg_price_array.get(0).unwrap(); // Now a f64
            let last_avg_price = avg_price_array.get(6).unwrap(); // Now a f64

            let change = first_avg_price - last_avg_price;
            Ok(change)
        } else {
            Ok(0.0)
        }
    }
    pub async fn delete_all_orders(&self, mode: OrderMode) -> Result<(), AppError> {
        let wfm = self.client.wfm.lock()?.clone();
        let settings = self.client.settings.lock()?.clone().live_scraper;
        let blacklist = settings.stock_item.blacklist.clone();
        self.client.send_message("item.deleting_orders", Some(json!({ "count": 0, "total": 0})));
        let mut current_orders = wfm.orders().get_my_orders().await?;

        let mut orders = vec![];

        if mode == OrderMode::Buy || mode == OrderMode::Both {
            orders.append(&mut current_orders.buy_orders);            
        }
        if mode == OrderMode::Sell || mode == OrderMode::Both {
            orders.append(&mut current_orders.sell_orders);
        }


        let mut current_index = 0;
        let total = orders.len();
        self.client.send_message("item.deleting_orders", Some(json!({ "count": 0, "total": total})));
        for order in orders {
            current_index += 1;
            self.client.send_message("item.deleting_orders", Some(json!({ "count": current_index, "total": total})));
            if self.client.is_running() == false {
                return Ok(());
            }
            // Check if item is in blacklist
            if blacklist.contains(&order.clone().item.unwrap().url_name) {
                continue;
            }
            wfm.orders()
                .delete(&order.id, "None", "None", "Any")
                .await?;
        }
        Ok(())
    }
    pub async fn get_buy_sell_overlap(&self) -> Result<DataFrame, AppError> {
        let settings = self.client.settings.lock()?.clone().live_scraper;
        let db = self.client.db.lock()?.clone();
        let df = self.client.price_scraper.lock()?.get_price_historys()?;
        let volume_threshold = settings.stock_item.volume_threshold;
        let range_threshold = settings.stock_item.range_threshold;
        let avg_price_cap = settings.stock_item.avg_price_cap;
        let price_shift_threshold = settings.stock_item.price_shift_threshold;
        let strict_whitelist = settings.stock_item.strict_whitelist;
        let whitelist = settings.stock_item.whitelist.clone();

        // Group by the "name" and "order_type" columns, and compute the mean of the other columns
        let averaged_df = df
            .clone()
            .lazy()
            .groupby(&["name", "order_type"])
            .agg(&[
                // List the other columns you want to average
                col("volume").mean().alias("volume"),
                col("min_price").mean().alias("min_price"),
                col("max_price").mean().alias("max_price"),
                col("range").mean().alias("range"),
                col("median").mean().alias("median"),
                col("avg_price").mean().alias("avg_price"),
                col("mod_rank").mean().alias("mod_rank"),
                col("item_id").first().alias("item_id"),
            ])
            .collect()
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;
        // Call the database to get the inventory names and DataFrame
        let inventory_names = db.stock_item().get_items_names().await?;
        let inventory_names_s = Series::new("desired_column_name", inventory_names);

        // Filters the DataFrame based on the given predicates and returns a new DataFrame.
        // The `volume_threshold` and `range_threshold` arguments are used to filter by volume and range.
        // The `inventory_names_s` argument is used to filter by name.
        // The `closed` order type is used to filter by order type.
        let filtered_df = averaged_df
            .clone()
            .lazy()
            .filter(
                col("order_type").eq(lit("closed")).and(
                    col("volume")
                        .gt(lit(volume_threshold))
                        .and(col("range").gt(lit(range_threshold)))
                        .or(col("name").is_in(lit(inventory_names_s.clone()))),
                ),
            )
            .collect()
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;

        // Sort by "range" in descending order
        let mut filtered_df = helper::sort_dataframe(filtered_df, "range", true)?;

        // If the DataFrame is empty, return an empty DataFrame
        if filtered_df.height() == 0 {
            return Ok(DataFrame::new(vec![
                Series::new("name", &[] as &[&str]),
                Series::new("minSell", &[] as &[f64]),
                Series::new("maxBuy", &[] as &[f64]),
                Series::new("overlap", &[] as &[f64]),
                Series::new("closedVol", &[] as &[f64]),
                Series::new("closedMin", &[] as &[f64]),
                Series::new("closedMax", &[] as &[f64]),
                Series::new("closedAvg", &[] as &[f64]),
                Series::new("closedMedian", &[] as &[f64]),
                Series::new("priceShift", &[] as &[f64]),
                Series::new("mod_rank", &[] as &[i32]),
                Series::new("item_id", &[] as &[&str]),
            ])
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?);
        }

        // Get the "name" column from the DataFrame
        let name_column = filtered_df
            .column("name")
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;

        // Create a new Series with the calculated week price shifts
        let week_price_shifts: Vec<f64> = name_column
            .utf8()
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?
            .into_iter()
            .filter_map(|opt_name| {
                opt_name.map(|name| self.get_week_increase(&df, name).unwrap_or(0.0))
            })
            .collect();

        let mut filtered_df = filtered_df
            .with_column(Series::new("weekPriceShift", week_price_shifts))
            .cloned()
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;

        // Handle the whitelist if it is strict or not
        let whitelist_s = Series::new("whitelist", whitelist);
        if strict_whitelist {
            filtered_df = filtered_df
                .lazy()
                .filter(col("name").is_in(lit(whitelist_s)))
                .collect()
                .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;
        } else {
            filtered_df = filtered_df
                .lazy()
                .filter(
                    col("avg_price")
                        .lt(lit(avg_price_cap))
                        .and(col("weekPriceShift").gt_eq(lit(price_shift_threshold)))
                        .or(col("name").is_in(lit(inventory_names_s)))
                        .or(col("name").is_in(lit(whitelist_s))),
                )
                .collect()
                .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;
        }

        // Extract unique names from filtered_df into a HashSet
        let name_set: HashSet<String> = HashSet::from_iter(
            match helper::get_column_values(filtered_df.clone(), None, "name", ColumnType::String)?
            {
                ColumnValues::String(values) => values,
                _ => return Err(AppError::new("LiveScraper", eyre!("Expected f64 values"))),
            },
        );
        let unique_names = name_set.into_iter().collect::<Vec<_>>();

        let unique_names_series = Series::new("name", unique_names.clone());
        let df_filtered = averaged_df
            .clone()
            .lazy()
            .filter(col("name").is_in(lit(unique_names_series.clone())))
            .collect()
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;

        // Start the creation of the buy_sell_overlap DataFrame
        let buy_sell_overlap = DataFrame::new(vec![unique_names_series])
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;

        // Get Order type "sell" and "buy" into separate DataFrames
        let mut order_sell_df = helper::filter_and_extract(
            df_filtered.clone(),
            Some(col("order_type").eq(lit("sell"))),
            vec!["name", "min_price"],
        )?;
        let order_sell_df = order_sell_df
            .rename("min_price", "minSell")
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;

        let mut order_buy_df = helper::filter_and_extract(
            df_filtered.clone(),
            Some(col("order_type").eq(lit("buy"))),
            vec!["name", "max_price"],
        )?;
        let order_buy_df = order_buy_df
            .rename("max_price", "maxBuy")
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;

        // Remove unnecessary columns
        let filtered_df = filtered_df.drop_many(&["range", "order_type"]);

        // Join the DataFrames together
        let buy_sell_overlap = buy_sell_overlap
            .inner_join(&order_sell_df, ["name"], ["name"])
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?
            .inner_join(&order_buy_df, ["name"], ["name"])
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?
            .inner_join(&filtered_df, ["name"], ["name"])
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;

        // Calculate the overlap
        let mut buy_sell_overlap: DataFrame = buy_sell_overlap
            .clone()
            .lazy()
            .fill_nan(lit(0.0).alias("maxBuy"))
            .fill_nan(lit(0.0).alias("minSell"))
            .with_column((col("maxBuy") - col("minSell")).alias("overlap"))
            .collect()
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;

        // Rename the columns
        let buy_sell_overlap = buy_sell_overlap
            .rename("volume", "closedVol")
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?
            .rename("min_price", "closedMin")
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?
            .rename("max_price", "closedMax")
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?
            .rename("avg_price", "closedAvg")
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?
            .rename("median", "closedMedian")
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?
            .rename("weekPriceShift", "priceShift")
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;

        return Ok(buy_sell_overlap.clone());
    }
    async fn get_my_order_information(
        &self,
        item_name: &str,
        df: &DataFrame,
    ) -> Result<(Option<String>, bool, i64, bool), AppError> {
        let orders_by_item = df
            .clone()
            .lazy()
            .filter(col("url_name").eq(lit(item_name)))
            .collect()
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;
        let id: Option<String> = None;
        let visibility = false;
        let price = 0;
        if orders_by_item.height() == 0 {
            return Ok((id, visibility, price, false));
        }
        let id =
            match helper::get_column_value(orders_by_item.clone(), None, "id", ColumnType::String)?
            {
                ColumnValue::String(values) => values,
                _ => return Err(AppError::new("LiveScraper", eyre!("Expected f64 values"))),
            };

        let visibility = match helper::get_column_value(
            orders_by_item.clone(),
            None,
            "visible",
            ColumnType::Bool,
        )? {
            ColumnValue::Bool(values) => values.unwrap_or(false),
            _ => return Err(AppError::new("LiveScraper", eyre!("Expected f64 values"))),
        };

        let price: i64 = match helper::get_column_value(
            orders_by_item.clone(),
            None,
            "platinum",
            ColumnType::I64,
        )? {
            ColumnValue::I64(values) => values.unwrap_or(0),
            _ => return Err(AppError::new("LiveScraper", eyre!("Expected f64 values"))),
        };
        Ok((id.clone(), visibility, price, true))
    }
    async fn restructure_live_order_df(
        &self,
        item_live_orders_df: &DataFrame,
    ) -> Result<(DataFrame, DataFrame, i64, i64, i64), AppError> {
        let in_game_name = self.client.auth.lock()?.clone().ingame_name;
        let buy_orders_df = item_live_orders_df
            .clone()
            .lazy()
            .filter(
                col("username")
                    .neq(lit(in_game_name.clone()))
                    .and(col("order_type").eq(lit("buy"))), // Add this line
            )
            .collect()
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;
        let buy_orders_df = helper::sort_dataframe(buy_orders_df, "platinum", true)?;

        let sell_orders_df = item_live_orders_df
            .clone()
            .lazy()
            .filter(
                col("username")
                    .neq(lit(in_game_name))
                    .and(col("order_type").eq(lit("sell"))), // Add this line
            )
            .collect()
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;
        let sell_orders_df = helper::sort_dataframe(sell_orders_df, "platinum", false)?;

        let mut lowest_price = 0;
        let mut highest_price = 0;

        let buyers = buy_orders_df.height() as i64;
        let sellers = sell_orders_df.height() as i64;
        if buyers > 0 {
            lowest_price = match helper::get_column_value(
                buy_orders_df.clone(),
                None,
                "platinum",
                ColumnType::I64,
            )? {
                ColumnValue::I64(values) => values.unwrap_or(0),
                _ => return Err(AppError::new("LiveScraper", eyre!("Expected f64 values"))),
            };
        }

        if sellers > 0 {
            highest_price = match helper::get_column_value(
                sell_orders_df.clone(),
                None,
                "platinum",
                ColumnType::I64,
            )? {
                ColumnValue::I64(values) => values.unwrap_or(0),
                _ => return Err(AppError::new("LiveScraper", eyre!("Expected f64 values"))),
            };
        }
        let range = highest_price - lowest_price;
        Ok((buy_orders_df, sell_orders_df, buyers, sellers, range))
    }

    fn is_item_blacklisted(&self, item_name: &str) -> Result<bool, AppError> {
        let settings = self.client.settings.lock()?.clone().live_scraper;
        let blacklist = settings.stock_item.blacklist.clone();
        let blacklist_s = Series::new("blacklist", blacklist);
        let blacklist_df = DataFrame::new(vec![blacklist_s]).unwrap();
        let blacklist_df = blacklist_df
            .lazy()
            .filter(col("blacklist").eq(lit(item_name)))
            .collect()
            .unwrap();
        if blacklist_df.height() == 0 {
            return Ok(false);
        }
        Ok(true)
    }

    fn knapsack(
        &self,
        items: Vec<(i64, f64, String, String)>,
        max_weight: i64,
    ) -> Result<
        (
            i64,
            Vec<(i64, f64, String, String)>,
            Vec<(i64, f64, String, String)>,
        ),
        AppError,
    > {
        let n = items.len();
        let mut dp = vec![vec![0; (max_weight + 1) as usize]; (n + 1) as usize];

        for i in 1..=n {
            for w in 1..=max_weight {
                let (weight, value, _, _) = items[i - 1];
                if weight <= w {
                    dp[i][w as usize] =
                        dp[i - 1][w as usize].max(dp[i - 1][(w - weight) as usize] + value as i64);
                } else {
                    dp[i][w as usize] = dp[i - 1][w as usize];
                }
            }
        }

        let mut selected_items = Vec::new();
        let mut unselected_items = Vec::new();
        let mut w = max_weight;
        for i in (0..n).rev() {
            if dp[i + 1][w as usize] != dp[i][w as usize] {
                selected_items.push(items[i].clone());
                w -= items[i].0;
            } else {
                unselected_items.push(items[i].clone());
            }
        }

        Ok((dp[n][max_weight as usize], selected_items, unselected_items))
    }
    async fn compare_live_orders_when_buying(
        &self,
        item_name: &str,
        item_id: &str,
        item_rank: Option<f64>,
        current_orders: DataFrame,
        item_live_orders_df: &DataFrame,
        item_stats: &DataFrame,
        inventory_df: &DataFrame,
    ) -> Result<Option<DataFrame>, AppError> {
        // Check if item is blacklisted
        if self.is_item_blacklisted(item_name)? {
            return Ok(None);
        }

        let settings = self.client.settings.lock()?.clone().live_scraper;
        let wfm = self.client.wfm.lock()?.clone();
        let mut current_orders = current_orders.clone();
        let avg_price_cap = settings.stock_item.avg_price_cap;
        let max_total_price_cap = settings.stock_item.max_total_price_cap;
        // Get the current orders for the item from the Warframe Market API
        let (order_id, visibility, price, active) = self
            .get_my_order_information(item_name, &current_orders)
            .await?;

        // Get all the live orders for the item from the Warframe Market API
        let (live_buy_orders_df, _live_sell_orders_df, buyers, sellers, price_range) =
            self.restructure_live_order_df(item_live_orders_df).await?;

        // Probably don't want to be looking at this item right now if there's literally nobody interested in selling it.
        if sellers == 0 {
            return Ok(None);
        }

        // Get the average price of the item from the Warframe Market API
        let item_closed_avg: f64 =
            match helper::get_column_value(item_stats.clone(), None, "closedAvg", ColumnType::F64)?
            {
                ColumnValue::F64(values) => values.unwrap_or(0.0),
                _ => return Err(AppError::new("LiveScraper", eyre!("Expected f64 values"))),
            };

        // If there are no buyers, and the average price is greater than 25p, then we should probably update our listing.
        if buyers == 0 && item_closed_avg > 25.0 {
            // If the item is worth more than 40p, then we should probably update our listing.
            let mut post_price = (price_range - 40).max((price_range / 3) - 1);

            if post_price > avg_price_cap as i64 {
                logger::info_con("LiveScraper",format!("Item {item_name} is higher than the price cap you set. cap: {avg_price_cap}, post_price: {post_price}").as_str());
                return Ok(None);
            }
            // If the item is worth less than 1p, then we should probably update our listing.
            if post_price < 1 {
                post_price = 1;
            }
            // If the order is active, then we should update it else we should post a new order.
            if active {
                self.client.send_message("item.buy.updating", Some(json!({ "name": item_name, "price": post_price})));
                wfm.orders()
                    .update(
                        order_id.clone().unwrap().as_str(),
                        post_price as i32,
                        1,
                        visibility,
                        item_name,
                        item_id,
                        "buy",
                    )
                    .await?;
                return Ok(None);
            } else {
                self.client.send_message("item.buy.creating", Some(json!({ "name": item_name, "price": post_price})));
                wfm.orders()
                    .create(item_name, item_id, "buy", post_price, 1, true, item_rank)
                    .await?;
                logger::info_con("LiveScraper",format!("Automatically Posted Visible Buy Order Item: {item_name}, ItemId: {item_id}, Price: {post_price}").as_str());
                return Ok(None);
            }
        } else if buyers == 0 {
            return Ok(None);
        }

        // Get highest buy order price
        let post_price: i64 = match helper::get_column_value(
            live_buy_orders_df.clone(),
            None,
            "platinum",
            ColumnType::I64,
        )? {
            ColumnValue::I64(values) => values.unwrap_or(0),
            _ => return Err(AppError::new("LiveScraper", eyre!("Expected i64 values"))),
        };

        // Get the average price of the item from the Warframe Market API
        let closed_avg_metric: f64 =
            match helper::get_column_value(item_stats.clone(), None, "closedAvg", ColumnType::F64)?
            {
                ColumnValue::F64(values) => values.unwrap_or(0.0) - post_price as f64,
                _ => return Err(AppError::new("LiveScraper", eyre!("Expected f64 values"))),
            };
        let potential_profit = closed_avg_metric - 1.0;

        // Check if the post price is greater than the average price cap and return if it is
        if post_price > avg_price_cap as i64 {
            logger::info_con("LiveScraper",format!("Item {item_name} is higher than the price cap you set. cap: {avg_price_cap}, post_price: {post_price}").as_str());
            return Ok(None);
        }
        // Get the owned value from the database
        let owned: i32 = match helper::get_column_value(
            inventory_df.clone(),
            Some(col("item_url").eq(lit(item_name))),
            "owned",
            ColumnType::I32,
        )? {
            ColumnValue::I32(values) => values.unwrap_or(0),
            _ => return Err(AppError::new("LiveScraper", eyre!("Expected f64 values"))),
        };

        if owned > 1 && ((closed_avg_metric as i64) < (25 * owned as i64)) {
            logger::info_con(
                "LiveScraper",
                format!("You're holding too many of this {item_name}! Not putting up a buy order.")
                    .as_str(),
            );
            if active {
                logger::info_con(
                    "LiveScraper",
                    format!("In fact you have a buy order up for this {item_name}! Deleting it.")
                        .as_str(),
                );
                self.client.send_message("item.buy.deleting", Some(json!({ "name": item_name})));
                wfm.orders()
                    .delete(
                        order_id.clone().unwrap().as_str(),
                        item_name,
                        item_id,
                        "buy",
                    )
                    .await?;
            }
            return Ok(None);
        }
        if ((closed_avg_metric as i64) >= 30 && price_range >= 15) || price_range >= 21 {
            if active {
                if price != post_price {
                    logger::info_con("LiveScraper", format!("Your current posting on this item {item_name} for {price} plat is not a good one. Updating to {post_price} plat.").as_str());
                    self.client.send_message("item.buy.updating", Some(json!({ "name": item_name, "price": post_price})));
                    wfm.orders()
                        .update(
                            order_id.clone().unwrap().as_str(),
                            post_price as i32,
                            1,
                            visibility,
                            item_name,
                            item_id,
                            "buy",
                        )
                        .await?;
                    let df = DataFrame::new(vec![
                        Series::new("url_name", vec![item_name]),
                        Series::new("platinum", vec![post_price]),
                        Series::new("potential_profit", vec![(post_price - price)]),
                    ])
                    .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;
                    let updatede = current_orders
                        .inner_join(&df, ["url_name"], ["url_name"])
                        .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;
                    return Ok(Some(updatede));
                } else {
                    self.client.send_message("item.buy.updating", Some(json!({ "name": item_name, "price": post_price})));
                    wfm.orders()
                        .update(
                            order_id.clone().unwrap().as_str(),
                            post_price as i32,
                            1,
                            visibility,
                            item_name,
                            item_id,
                            "buy",
                        )
                        .await?;
                    logger::info_con("LiveScraper", format!("Your current (possibly hidden) posting on this item {item_name} for {price} plat is a good one. Recommend to make visible.").as_str());
                    return Ok(None);
                }
            } else {
                let mut buy_orders_list: Vec<(i64, f64, String, String)> = vec![];
                // Create a Vec of Tuples from the DataFrame of current orders
                if current_orders.shape().0 != 0 {
                    // Convert to Vec of Tuples
                    let platinum_values = match helper::get_column_values(
                        current_orders.clone(),
                        None,
                        "platinum",
                        ColumnType::I64,
                    )? {
                        ColumnValues::I64(values) => values,
                        _ => {
                            return Err(AppError::new("LiveScraper", eyre!("Expected i64 values")))
                        }
                    };
                    let potential_profit_values = match helper::get_column_values(
                        current_orders.clone(),
                        None,
                        "potential_profit",
                        ColumnType::F64,
                    )? {
                        ColumnValues::F64(values) => values,
                        _ => {
                            return Err(AppError::new("LiveScraper", eyre!("Expected f64 values")))
                        }
                    };

                    let url_name_values = match helper::get_column_values(
                        current_orders.clone(),
                        None,
                        "url_name",
                        ColumnType::String,
                    )? {
                        ColumnValues::String(values) => values,
                        _ => {
                            return Err(AppError::new(
                                "LiveScraper",
                                eyre!("Expected string values"),
                            ))
                        }
                    };
                    let id_values = match helper::get_column_values(
                        current_orders.clone(),
                        None,
                        "id",
                        ColumnType::String,
                    )? {
                        ColumnValues::String(values) => values,
                        _ => {
                            return Err(AppError::new(
                                "LiveScraper",
                                eyre!("Expected string values"),
                            ))
                        }
                    };
                    buy_orders_list = platinum_values
                        .into_iter()
                        .zip(potential_profit_values.into_iter())
                        .zip(url_name_values.into_iter())
                        .zip(id_values.into_iter())
                        .map(|(((platinum, profit), url_name), id)| {
                            (platinum, profit, url_name, id)
                        })
                        .collect();
                }
                buy_orders_list.append(&mut vec![(
                    post_price,
                    potential_profit,
                    item_name.to_string(),
                    "".to_string(),
                )]);

                let (_max_profit, selected_buy_orders, unselected_buy_orders) =
                    self.knapsack(buy_orders_list, max_total_price_cap as i64)?;

                let selected_item_names: Vec<String> = selected_buy_orders
                    .iter()
                    .map(|order| order.2.clone())
                    .collect();

                if selected_item_names.contains(&item_name.to_string()) {
                    if !unselected_buy_orders.is_empty() {
                        let unselected_item_names: Vec<String> = unselected_buy_orders
                            .iter()
                            .map(|order| order.2.clone())
                            .collect();
                        logger::info_con("LiveScraper",format!("Item {} is not as optimal as other items. Deleting buy orders for {:?}", item_name, unselected_item_names).as_str());

                        current_orders = current_orders
                            .lazy()
                            .filter(
                                col("url_name")
                                    .is_in(lit(Series::new(
                                        "unselected_url_name",
                                        unselected_item_names.clone(),
                                    )))
                                    .not(),
                            )
                            .collect()
                            .map_err(|e| {
                                AppError::new(
                                    "LiveScraper",
                                    eyre!(
                                        "{:?}, {:?}, Item: {:?}",
                                        e.to_string(),
                                        unselected_item_names.clone(),
                                        item_name
                                    ),
                                )
                            })?;

                        for unselected_item in &unselected_buy_orders {
                            self.client.send_message("item.buy.deleting", Some(json!({ "name": unselected_item.2})));
                            wfm.orders()
                                .delete(unselected_item.3.as_str(), item_name, item_id, "buy")
                                .await?;
                            logger::debug_con(
                                "component",
                                format!(
                                    "DELETED BUY order for {} since it is not as optimal",
                                    unselected_item.2
                                )
                                .as_str(),
                            );
                        }
                    }
                    self.client.send_message("item.buy.creating", Some(json!({ "name": item_name, "price": post_price})));
                    let new_order = wfm
                        .orders()
                        .create(item_name, item_id, "buy", post_price, 1, true, item_rank)
                        .await?;
                    let current_orders =
                        self.get_new_buy_data(current_orders.clone(), new_order, item_closed_avg)?;
                    return Ok(Some(current_orders));
                } else {
                    logger::info_con("LiveScraper",format!("Item {item_name} is too expensive or less optimal than current listings").as_str());
                }
            }
        } else if active {
            logger::info_con("LiveScraper",format!("Item {item_name} Not a good time to have an order up on this item. Deleted buy order for {price}").as_str());
            self.client.send_message("item.buy.deleting", Some(json!({ "name": item_name})));
            wfm.orders()
                .delete(
                    order_id.clone().unwrap().as_str(),
                    item_name,
                    item_id,
                    "buy",
                )
                .await?;
        }

        Ok(None)
    }
    async fn compare_live_orders_when_selling(
        &self,
        item_name: &str,
        item_id: &str,
        item_rank: Option<f64>,
        current_orders: DataFrame,
        item_live_orders_df: &DataFrame,
        _item_stats: &DataFrame,
        _inventory_df: &DataFrame,
    ) -> Result<(), AppError> {
        let wfm = self.client.wfm.lock()?.clone();
        let db = self.client.db.lock()?.clone();

        // Get the current orders for the item from the Warframe Market API
        let (order_id, visibility, price, active) = self
            .get_my_order_information(item_name, &current_orders)
            .await?;

        let inventory_names = db.stock_item().get_items_names().await?;

        if !inventory_names.contains(&item_name.to_string()) && !active {
            return Ok(());
        } else if !inventory_names.contains(&item_name.to_string()) {
            self.client.send_message("item.sell.deleting", Some(json!({ "name": item_name})));
            db.stock_item()
                .update_by_url(
                    item_name,
                    None,
                    None,
                    None,
                    Some("to_low_profit".to_string()),
                    None,
                )
                .await?;
            wfm.orders()
                .delete(
                    order_id.clone().unwrap().as_str(),
                    item_name,
                    item_id,
                    "sell",
                )
                .await?;
            logger::info_con(
                "LiveScraper",
                format!(
                    "Item {item_name} is not in your inventory. Deleted sell order for {price}"
                )
                .as_str(),
            );
            return Ok(());
        }

        // Get Invantory Data from the database
        let stock_item = db
            .stock_item()
            .get_item_by_url_name(item_name)
            .await?
            .unwrap();

        // Get all the live orders for the item from the Warframe Market API
        let (_live_buy_orders_df, live_sell_orders_df, _buyers, sellers, _price_range) =
            self.restructure_live_order_df(item_live_orders_df).await?;

        // Get the average price of the item.
        let bought_avg_price =
            (stock_item.price * stock_item.owned as f64 / stock_item.owned as f64) as i64;

        // Get the quantity of owned item.
        let quantity = stock_item.owned as i64;

        // Get the minimum price of the item.
        let minimum_price = stock_item.minium_price;

        // If there are no buyers, update order to be 30p above average price
        if sellers == 0 {
            let mut post_price = (bought_avg_price + 30) as i64;
            if minimum_price.is_some() && post_price < minimum_price.unwrap() as i64 {
                post_price = minimum_price.unwrap() as i64;
            }

            db.stock_item()
                .update_by_url(
                    item_name,
                    None,
                    None,
                    Some(post_price as i32),
                    Some("no_buyers".to_string()),
                    None,
                )
                .await?;
            if active {
                self.client.send_message("item.sell.deleting", Some(json!({ "name": item_name})));
                wfm.orders()
                    .update(
                        order_id.clone().unwrap().as_str(),
                        post_price as i32,
                        quantity as i32,
                        visibility,
                        item_name,
                        item_id,
                        "sell",
                    )
                    .await?;
                return Ok(());
            } else {
                wfm.orders()
                    .create(
                        item_name, item_id, "sell", post_price, quantity, true, item_rank,
                    )
                    .await?;
                return Ok(());
            }
        }

        // Get the platinum values from the DataFrame of live sell orders
        let post_prices: Vec<i64> = match helper::get_column_values(
            live_sell_orders_df.clone(),
            None,
            "platinum",
            ColumnType::I64,
        )? {
            ColumnValues::I64(values) => values,
            _ => return Err(AppError::new("LiveScraper", eyre!("Expected i64 values"))),
        };

        // Get lowest buy order price from the DataFrame of live sell orders
        let mut post_price = post_prices.get(0).unwrap_or(&0).clone();

        // Get the profit from the current order
        let profit = post_price - bought_avg_price as i64;
        
        if profit <= -10 {
            // Only update the database if the item is not already marked as to_low_profit
            if stock_item.status != "to_low_profit" {
                db.stock_item()
                    .update_by_url(
                        item_name,
                        None,
                        None,
                        Some(-1),
                        Some("to_low_profit".to_string()),
                        None,
                    )
                    .await?;
            }
            logger::info_con(
                "LiveScraper",
                format!("Item {item_name} is too cheap. Not putting up a sell order.").as_str(),
            );
            if active {
                self.client.send_message("item.sell.deleting", Some(json!({ "name": item_name})));
                wfm.orders()
                    .delete(
                        order_id.clone().unwrap().as_str(),
                        item_name,
                        item_id,
                        "sell",
                    )
                    .await?;
            }
            return Ok(());
        }

        if post_price + 10 > post_price && sellers >= 2 {
            post_price = (bought_avg_price + 10).max(post_prices.get(0).unwrap_or(&0).clone());
        } else {
            post_price = (bought_avg_price + 10).max(post_price);
        }
        if minimum_price.is_some() && post_price < minimum_price.unwrap() as i64 {
            post_price = minimum_price.unwrap() as i64;
        }
        if active {
            if price != post_price {
                self.client.send_message("item.sell.updating", Some(json!({ "name": item_name, "price": post_price})));
                wfm.orders()
                    .update(
                        order_id.clone().unwrap().as_str(),
                        post_price as i32,
                        quantity as i32,
                        visibility,
                        item_name,
                        item_id,
                        "sell",
                    )
                    .await?;
                db.stock_item()
                    .update_by_url(
                        item_name,
                        None,
                        None,
                        Some(post_price as i32),
                        Some("live".to_string()),
                        None,
                    )
                    .await?;
                logger::info_con(
                    "LiveScraper",
                    format!(
                        "Automatically updated order {} for {} from {} to {} plat",
                        order_id.unwrap_or("None".to_string()),
                        item_name,
                        price,
                        post_price
                    )
                    .as_str(),
                );
                return Ok(());
            } else {
                logger::info_con("LiveScraper", format!("Your current (possibly hidden) posting on this item {item_name} for {price} plat is a good one. Recommend to make visible.").as_str());
                return Ok(());
            }
        } else {
            self.client.send_message("item.sell.creating", Some(json!({ "name": item_name, "price": post_price})));
            wfm.orders()
                .create(
                    item_name, item_id, "sell", post_price, quantity, true, item_rank,
                )
                .await?;
            db.stock_item()
                .update_by_url(
                    item_name,
                    None,
                    None,
                    Some(post_price as i32),
                    Some("live".to_string()),
                    None,
                )
                .await?;
            logger::info_con("LiveScraper",format!("Automatically Posted Visible Sell Order Item: {item_name}, ItemId: {item_id}, Price: {post_price}").as_str());
        }
        Ok(())
    }
    fn get_new_buy_data(
        &self,
        mut current_orders: DataFrame,
        order: Order,
        item_closed_avg: f64,
    ) -> Result<DataFrame, AppError> {
        let mut order_df = self
            .client
            .wfm
            .lock()?
            .orders()
            .convet_order_to_datafream(order.clone())?;
        order_df = order_df
            .with_column(Series::new(
                "potential_profit",
                vec![item_closed_avg - order.platinum.clone() as f64],
            ))
            .cloned()
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?
            .with_column(Series::new("closedAvg", vec![item_closed_avg]))
            .cloned()
            .map_err(|e| AppError::new("LiveScraper", eyre!(e.to_string())))?;
        current_orders = current_orders.drop("username").unwrap();
        Ok(helper::merge_dataframes(vec![current_orders, order_df])?)
    }
}
